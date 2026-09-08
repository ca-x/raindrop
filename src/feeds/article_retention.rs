use super::{FeedRepository, FeedRetentionError};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArticleRetentionSettings {
    pub enabled: bool,
    pub retention_days: u16,
}
impl Default for ArticleRetentionSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            retention_days: 30,
        }
    }
}
impl ArticleRetentionSettings {
    pub async fn load(database: &DatabaseConnection) -> Result<Self, FeedRetentionError> {
        let row = database
            .query_one(Statement::from_string(
                database.get_database_backend(),
                "SELECT enabled, retention_days FROM article_retention_settings WHERE id = 1"
                    .to_owned(),
            ))
            .await?;
        match row {
            None => Ok(Self::default()),
            Some(row) => {
                let days: i32 = row.try_get("", "retention_days")?;
                let settings = Self {
                    enabled: row.try_get("", "enabled")?,
                    retention_days: u16::try_from(days)
                        .map_err(|_| FeedRetentionError::CorruptData)?,
                };
                settings.validate()?;
                Ok(settings)
            }
        }
    }
    pub fn validate(&self) -> Result<(), FeedRetentionError> {
        if !(1..=3650).contains(&self.retention_days) {
            return Err(FeedRetentionError::InvalidRequest);
        }
        Ok(())
    }
    pub async fn save(&self, database: &DatabaseConnection) -> Result<(), FeedRetentionError> {
        self.validate()?;
        // This instance-wide maintenance feature is currently exposed for SQLite only.
        if database.get_database_backend() != DatabaseBackend::Sqlite {
            return Err(FeedRetentionError::InvalidRequest);
        }
        database.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "INSERT INTO article_retention_settings (id,enabled,retention_days) VALUES (1,?,?)
             ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled, retention_days=excluded.retention_days",
            [self.enabled.into(), i32::from(self.retention_days).into()])).await?;
        Ok(())
    }
}

impl FeedRepository {
    /// Remove only aged articles read by every subscriber. Explicit unread overrides, any star,
    /// and articles with saved/generated AI work are protected. Tombstones prevent resurrection
    /// on the next feed fetch, while occupying only the feed and identity hashes.
    pub async fn purge_read_articles(&self) -> Result<usize, FeedRetentionError> {
        let db = self.connection();
        if db.get_database_backend() != DatabaseBackend::Sqlite {
            return Ok(0);
        }
        let settings = ArticleRetentionSettings::load(db).await?;
        if !settings.enabled {
            return Ok(0);
        }
        let cutoff =
            OffsetDateTime::now_utc() - time::Duration::days(i64::from(settings.retention_days));
        let tx = db.begin().await?;
        let rows = tx
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT id, feed_id, identity_hash FROM entries e WHERE inserted_at < ?
             AND NOT EXISTS (SELECT 1 FROM entry_states es WHERE es.entry_id=e.id
               AND (es.is_starred=TRUE OR es.read_override=FALSE))
             AND EXISTS (SELECT 1 FROM subscriptions s WHERE s.feed_id=e.feed_id
               AND e.feed_sequence > s.start_sequence)
             AND NOT EXISTS (SELECT 1 FROM subscriptions s
               LEFT JOIN entry_states es ON es.user_id=s.user_id AND es.entry_id=e.id
               WHERE s.feed_id=e.feed_id AND e.feed_sequence > s.start_sequence
               AND COALESCE(es.read_override, e.feed_sequence <= s.read_through_sequence)=FALSE)
             AND NOT EXISTS (SELECT 1 FROM content_jobs j WHERE j.entry_id=e.id)
             ORDER BY inserted_at, id LIMIT 100",
                [cutoff.into()],
            ))
            .await?;
        let count = rows.len();
        for row in rows {
            let id: String = row.try_get("", "id")?;
            let feed_id: String = row.try_get("", "feed_id")?;
            let hash: String = row.try_get("", "identity_hash")?;
            tx.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
                "INSERT INTO retired_entry_identities (feed_id, identity_hash) VALUES (?,?) ON CONFLICT DO NOTHING",
                [feed_id.into(), hash.into()])).await?;
            tx.execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "DELETE FROM entries WHERE id=?",
                [id.into()],
            ))
            .await?;
        }
        tx.commit().await?;
        Ok(count)
    }
}
