use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand_core::{OsRng, RngCore};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbBackend, DbErr, QueryResult, Statement,
    TransactionTrait, Value,
};
use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TokenItem {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub created_at: String,
    pub expires_at: String,
    pub last_used_at: Option<String>,
}

pub(crate) fn statement(backend: DbBackend, sql: &str, values: Vec<Value>) -> Statement {
    let mut index = 0;
    let sql = if backend == DbBackend::Postgres {
        sql.chars()
            .map(|character| {
                if character == '?' {
                    index += 1;
                    format!("${index}")
                } else {
                    character.to_string()
                }
            })
            .collect::<String>()
    } else {
        sql.to_owned()
    };
    Statement::from_sql_and_values(backend, sql, values)
}

fn timestamp(value: i64) -> Result<String, DbErr> {
    OffsetDateTime::from_unix_timestamp(value)
        .ok()
        .and_then(|value| value.format(&Rfc3339).ok())
        .ok_or_else(|| DbErr::Custom("invalid MCP timestamp".into()))
}

fn item(row: QueryResult) -> Result<TokenItem, DbErr> {
    Ok(TokenItem {
        id: row.try_get("", "id")?,
        name: row.try_get("", "name")?,
        prefix: row.try_get("", "prefix")?,
        created_at: timestamp(row.try_get("", "created_at")?)?,
        expires_at: timestamp(row.try_get("", "expires_at")?)?,
        last_used_at: row
            .try_get::<Option<i64>>("", "last_used_at")?
            .map(timestamp)
            .transpose()?,
    })
}

pub(crate) async fn list(
    database: &DatabaseConnection,
    user: &str,
) -> Result<Vec<TokenItem>, DbErr> {
    database.query_all(statement(database.get_database_backend(), "SELECT id,name,prefix,created_at,expires_at,last_used_at FROM mcp_tokens WHERE user_id = ? ORDER BY created_at,id", vec![user.into()]))
        .await?.into_iter().map(item).collect()
}

pub(crate) async fn create(
    database: &DatabaseConnection,
    user: &str,
    name: &str,
    days: i64,
) -> Result<Option<(String, TokenItem)>, DbErr> {
    let transaction = database.begin().await?;
    let backend = database.get_database_backend();
    let lock = if backend == DbBackend::Sqlite {
        "UPDATE users SET is_disabled = is_disabled WHERE id = ? AND is_disabled = FALSE"
    } else {
        "SELECT id FROM users WHERE id = ? AND is_disabled = FALSE FOR UPDATE"
    };
    let active = if backend == DbBackend::Sqlite {
        transaction
            .execute(statement(backend, lock, vec![user.into()]))
            .await?
            .rows_affected()
            == 1
    } else {
        transaction
            .query_one(statement(backend, lock, vec![user.into()]))
            .await?
            .is_some()
    };
    if !active {
        return Err(DbErr::Custom("MCP user unavailable".into()));
    }
    let count = transaction
        .query_one(statement(
            backend,
            "SELECT COUNT(*) AS count FROM mcp_tokens WHERE user_id = ?",
            vec![user.into()],
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("MCP count unavailable".into()))?
        .try_get::<i64>("", "count")?;
    if count >= 50 {
        return Ok(None);
    }
    let mut entropy = [0_u8; 32];
    OsRng
        .try_fill_bytes(&mut entropy)
        .map_err(|_| DbErr::Custom("token entropy unavailable".into()))?;
    let token = format!("rdmcp_{}", URL_SAFE_NO_PAD.encode(entropy));
    let hash = blake3::hash(token.as_bytes()).to_hex().to_string();
    let id = Uuid::new_v4().to_string();
    let prefix = token[..14].to_owned();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let expires = now + days * 86400;
    transaction.execute(statement(backend, "INSERT INTO mcp_tokens (id,user_id,name,prefix,token_hash,created_at,expires_at) VALUES (?,?,?,?,?,?,?)", vec![id.clone().into(), user.into(), name.into(), prefix.clone().into(), hash.into(), now.into(), expires.into()])).await?;
    transaction.commit().await?;
    Ok(Some((
        token,
        TokenItem {
            id,
            name: name.into(),
            prefix,
            created_at: timestamp(now)?,
            expires_at: timestamp(expires)?,
            last_used_at: None,
        },
    )))
}

pub(crate) async fn revoke(
    database: &DatabaseConnection,
    user: &str,
    id: &str,
) -> Result<bool, DbErr> {
    Ok(database
        .execute(statement(
            database.get_database_backend(),
            "DELETE FROM mcp_tokens WHERE id = ? AND user_id = ?",
            vec![id.into(), user.into()],
        ))
        .await?
        .rows_affected()
        > 0)
}

pub(crate) async fn authenticate(
    database: &DatabaseConnection,
    token: &str,
) -> Result<Option<String>, DbErr> {
    if token.len() != 49
        || !token.starts_with("rdmcp_")
        || URL_SAFE_NO_PAD.decode(&token[6..]).is_err()
    {
        return Ok(None);
    }
    let hash = blake3::hash(token.as_bytes()).to_hex().to_string();
    let backend = database.get_database_backend();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let row = database.query_one(statement(backend, "SELECT t.id,t.user_id FROM mcp_tokens t JOIN users u ON u.id=t.user_id WHERE t.token_hash = ? AND t.expires_at > ? AND u.is_disabled = FALSE", vec![hash.into(), now.into()])).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let id: String = row.try_get("", "id")?;
    database
        .execute(statement(
            backend,
            "UPDATE mcp_tokens SET last_used_at = ? WHERE id = ? AND expires_at > ?",
            vec![now.into(), id.clone().into(), now.into()],
        ))
        .await?;
    if database
        .query_one(statement(
            backend,
            "SELECT id FROM mcp_tokens WHERE id = ? AND expires_at > ?",
            vec![id.into(), now.into()],
        ))
        .await?
        .is_none()
    {
        return Ok(None);
    }
    Ok(Some(row.try_get("", "user_id")?))
}
