use std::{
    collections::HashSet,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use reqwest::Method;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use serde::{Deserialize, Serialize};

use crate::{
    api::{Api, ApiError, Subscription},
    local::Local,
};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderRef {
    pub(crate) provider_id: String,
    pub(crate) display_name: String,
    pub(crate) kind: String,
    pub(crate) model: String,
    pub(crate) is_enabled: bool,
    pub(crate) revision: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiConfig {
    pub(crate) plugin_state: String,
    pub(crate) config: Option<AiOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiOptions {
    pub(crate) revision: u64,
    pub(crate) is_enabled: bool,
    pub(crate) summary: AiAction,
    pub(crate) translation: AiAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiAction {
    pub(crate) enabled: bool,
    pub(crate) provider_id: String,
    pub(crate) max_output_tokens: u32,
    pub(crate) style: Option<String>,
    pub(crate) default_target_locale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Profile {
    pub(crate) id: String,
    pub(crate) address: String,
    pub(crate) user_id: String,
    pub(crate) username: String,
    pub(crate) providers: Vec<ProviderRef>,
    pub(crate) ai: AiConfig,
    pub(crate) last_import: Option<u64>,
}

impl Profile {
    pub(crate) fn label(&self) -> String {
        let host = url::Url::parse(&self.address)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_default();
        format!("{} · {}", self.username, host)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Link {
    profile_id: String,
    remote_id: String,
    local_id: String,
    feed_hash: String,
    remote: Subscription,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum IntentStatus {
    Ready,
    Unknown,
    Confirmed,
    Canceled,
}

impl IntentStatus {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Ready => "等待推送",
            Self::Unknown => "结果未知 · 需核对",
            Self::Confirmed => "已推送",
            Self::Canceled => "已取消",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Intent {
    pub(crate) id: String,
    pub(crate) profile_id: String,
    pub(crate) local_id: String,
    pub(crate) url: String,
    pub(crate) status: IntentStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Journal {
    schema_version: u32,
    pub(crate) profiles: Vec<Profile>,
    pub(crate) intents: Vec<Intent>,
    links: Vec<Link>,
    #[serde(default)]
    pending_imports: HashSet<(String, String)>,
    suppressions: HashSet<(String, String)>,
}

impl Default for Journal {
    fn default() -> Self {
        Self {
            schema_version: 1,
            profiles: Vec::new(),
            intents: Vec::new(),
            links: Vec::new(),
            pending_imports: HashSet::new(),
            suppressions: HashSet::new(),
        }
    }
}

impl Journal {
    pub(crate) fn load(local: &Local) -> Result<Self, ApiError> {
        local.block_on(async {
            local.database.execute_unprepared("CREATE TABLE IF NOT EXISTS desktop_sync_state (id INTEGER PRIMARY KEY CHECK(id = 1), document TEXT NOT NULL)")
                .await.map_err(|_| ApiError::LocalData)?;
            let row = local.database.query_one(Statement::from_string(DbBackend::Sqlite, "SELECT document FROM desktop_sync_state WHERE id = 1"))
                .await.map_err(|_| ApiError::LocalData)?;
            match row {
                Some(row) => {
                    let document: String = row.try_get("", "document").map_err(|_| ApiError::LocalData)?;
                    if document.len() > 8 * 1024 * 1024 { return Err(ApiError::LocalData); }
                    let journal: Self = serde_json::from_str(&document).map_err(|_| ApiError::LocalData)?;
                    if journal.schema_version != 1 || journal.profiles.len() > 32 || journal.intents.len() > 20_000 || journal.pending_imports.len() > 20_000 {
                        return Err(ApiError::LocalData);
                    }
                    Ok(journal)
                }
                None => Ok(Self::default()),
            }
        })
    }

    fn save(&self, local: &Local) -> Result<(), ApiError> {
        let document = serde_json::to_string(self).map_err(|_| ApiError::LocalData)?;
        if document.len() > 8 * 1024 * 1024 || self.pending_imports.len() > 20_000 {
            return Err(ApiError::LocalData);
        }
        local.block_on(local.database.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "INSERT INTO desktop_sync_state(id, document) VALUES(1, ?) ON CONFLICT(id) DO UPDATE SET document = excluded.document", [document.into()])))
            .map_err(|_| ApiError::LocalData)?;
        Ok(())
    }
}

fn local_core(api: &Api) -> Result<Arc<Local>, ApiError> {
    api.local_core().ok_or(ApiError::LocalData)
}

fn feed_hash(url: &str) -> Result<String, ApiError> {
    raindrop::feeds::FeedUrlPolicy::new(false)
        .normalize(url)
        .map(|normalized| normalized.url_hash().to_owned())
        .map_err(|_| ApiError::Response)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn profiles(api: &Api) -> Result<Journal, ApiError> {
    let local = local_core(api)?;
    let _guard = local.sync_lock.lock().map_err(|_| ApiError::LocalData)?;
    Journal::load(&local)
}

pub(crate) fn connect(api: &Api, remote: &Api, username: String) -> Result<Profile, ApiError> {
    let local = local_core(api)?;
    let _guard = local.sync_lock.lock().map_err(|_| ApiError::LocalData)?;
    let mut journal = Journal::load(&local)?;
    let address = remote.address();
    if let Some(profile) = journal
        .profiles
        .iter()
        .find(|profile| profile.address == address && profile.user_id == remote.user_id)
    {
        return Ok(profile.clone());
    }
    if journal.profiles.len() >= 32 {
        return Err(ApiError::Conflict);
    }
    let profile = Profile {
        id: uuid::Uuid::new_v4().to_string(),
        address,
        user_id: remote.user_id.clone(),
        username,
        providers: Vec::new(),
        ai: AiConfig::default(),
        last_import: None,
    };
    journal.profiles.push(profile.clone());
    journal.save(&local)?;
    Ok(profile)
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ImportReport {
    pub(crate) added: usize,
    pub(crate) linked: usize,
    pub(crate) changed: usize,
    pub(crate) suppressed: usize,
}

pub(crate) fn import(api: &Api, remote: &Api, profile_id: &str) -> Result<ImportReport, ApiError> {
    let local = local_core(api)?;
    let _guard = local.sync_lock.lock().map_err(|_| ApiError::LocalData)?;
    let mut journal = Journal::load(&local)?;
    let profile = journal
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .cloned()
        .ok_or(ApiError::Response)?;
    verify_identity(&profile, remote)?;
    let mut subscriptions = remote.subscriptions()?;
    let mut ids = HashSet::new();
    let mut hashes = HashSet::new();
    for subscription in &subscriptions {
        let hash = feed_hash(&subscription.feed_url)?;
        if !hashes.insert(hash)
            || !ids.insert(subscription.subscription_id.clone())
            || subscription.subscription_id.is_empty()
            || subscription.subscription_id.len() > 256
            || subscription
                .title_override
                .as_ref()
                .is_some_and(|title| title.len() > 200 || title.chars().any(char::is_control))
        {
            return Err(ApiError::Response);
        }
    }
    #[derive(Deserialize)]
    struct Providers {
        items: Vec<ProviderRef>,
    }
    #[derive(Deserialize)]
    struct Categories {
        items: Vec<Category>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Category {
        category_id: String,
        title: String,
    }
    let providers: Providers = remote.read("api/v1/ai/providers")?;
    let ai: AiConfig = remote.read("api/v1/ai/config")?;
    let categories: Categories = remote.read("api/v1/categories")?;
    let existing = api.subscriptions()?;
    let existing_ids: HashSet<_> = existing
        .iter()
        .map(|subscription| subscription.subscription_id.as_str())
        .collect();
    let mut report = ImportReport::default();
    subscriptions.sort_by(|left, right| {
        (left.position, &left.subscription_id).cmp(&(right.position, &right.subscription_id))
    });
    let mut pending = Vec::new();
    for remote_subscription in subscriptions {
        let hash = feed_hash(&remote_subscription.feed_url)?;
        if journal
            .suppressions
            .contains(&(profile_id.to_owned(), hash.clone()))
        {
            report.suppressed += 1;
            continue;
        }
        if let Some(link) = journal.links.iter().find(|link| {
            link.profile_id == profile_id && link.remote_id == remote_subscription.subscription_id
        }) {
            if !existing_ids.contains(link.local_id.as_str()) {
                journal.suppressions.insert((profile_id.to_owned(), hash));
                journal.save(&local)?;
                report.suppressed += 1;
            } else if link.feed_hash != hash
                || link.remote.title_override != remote_subscription.title_override
                || link.remote.category_id != remote_subscription.category_id
            {
                report.changed += 1;
            }
            continue;
        }
        pending.push(remote_subscription);
    }
    if !pending.is_empty() {
        for subscription in &pending {
            journal
                .pending_imports
                .insert((profile_id.to_owned(), feed_hash(&subscription.feed_url)?));
        }
        journal.save(&local)?;
        let mut document = String::from("<opml version=\"2.0\"><body>");
        for subscription in &pending {
            let category = subscription.category_id.as_ref().and_then(|id| {
                categories
                    .items
                    .iter()
                    .find(|category| &category.category_id == id)
            });
            if let Some(category) = category {
                let digest =
                    blake3::hash(format!("{profile_id}:{}", category.category_id).as_bytes())
                        .to_hex();
                let title = format!(
                    "{} · {}",
                    category
                        .title
                        .chars()
                        .filter(|character| !character.is_control())
                        .take(32)
                        .collect::<String>(),
                    &digest[..16]
                );
                document.push_str(&format!(
                    "<outline text=\"{}\">",
                    quick_xml::escape::escape(&title)
                ));
            }
            document.push_str(&format!(
                "<outline type=\"rss\" xmlUrl=\"{}\"",
                quick_xml::escape::escape(&subscription.feed_url)
            ));
            if let Some(title) = &subscription.title_override {
                document.push_str(&format!(" title=\"{}\"", quick_xml::escape::escape(title)));
            }
            document.push_str("/>");
            if category.is_some() {
                document.push_str("</outline>");
            }
        }
        document.push_str("</body></opml>");
        report.added = local.import_opml(document.as_bytes())?;
        report.linked = pending.len().saturating_sub(report.added);
    }
    let imported = api.subscriptions()?;
    for remote_subscription in pending {
        let hash = feed_hash(&remote_subscription.feed_url)?;
        let created = imported
            .iter()
            .find(|subscription| feed_hash(&subscription.feed_url).ok().as_ref() == Some(&hash))
            .ok_or(ApiError::LocalData)?;
        journal.links.push(Link {
            profile_id: profile_id.to_owned(),
            remote_id: remote_subscription.subscription_id.clone(),
            local_id: created.subscription_id.clone(),
            feed_hash: hash.clone(),
            remote: remote_subscription,
        });
        journal
            .pending_imports
            .remove(&(profile_id.to_owned(), hash));
    }
    let profile = journal
        .profiles
        .iter_mut()
        .find(|profile| profile.id == profile_id)
        .ok_or(ApiError::Response)?;
    profile.providers = providers.items;
    profile.ai = ai;
    profile.last_import = Some(now());
    journal.save(&local)?;
    Ok(report)
}

fn verify_identity(profile: &Profile, remote: &Api) -> Result<(), ApiError> {
    if profile.address != remote.address()
        || profile.user_id != remote.user_id
        || remote.user_id.is_empty()
    {
        return Err(ApiError::Unauthorized);
    }
    Ok(())
}

pub(crate) fn queue_push(
    api: &Api,
    profile_id: &str,
    subscription_id: &str,
) -> Result<(), ApiError> {
    let local = local_core(api)?;
    let _guard = local.sync_lock.lock().map_err(|_| ApiError::LocalData)?;
    let mut journal = Journal::load(&local)?;
    if !journal
        .profiles
        .iter()
        .any(|profile| profile.id == profile_id)
    {
        return Err(ApiError::Response);
    }
    let subscription = api
        .subscriptions()?
        .into_iter()
        .find(|subscription| subscription.subscription_id == subscription_id)
        .ok_or(ApiError::Response)?;
    if journal.intents.iter().any(|intent| {
        intent.profile_id == profile_id
            && intent.local_id == subscription_id
            && intent.status != IntentStatus::Canceled
    }) {
        return Err(ApiError::Conflict);
    }
    if journal.intents.len() >= 20_000 {
        return Err(ApiError::Conflict);
    }
    journal.intents.push(Intent {
        id: uuid::Uuid::new_v4().to_string(),
        profile_id: profile_id.to_owned(),
        local_id: subscription_id.to_owned(),
        url: subscription.feed_url,
        status: IntentStatus::Ready,
    });
    journal.save(&local)
}

pub(crate) fn push_pending(api: &Api, remote: &Api, profile_id: &str) -> Result<(), ApiError> {
    let local = local_core(api)?;
    let _guard = local.sync_lock.lock().map_err(|_| ApiError::LocalData)?;
    let mut journal = Journal::load(&local)?;
    let profile = journal
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .ok_or(ApiError::Response)?;
    verify_identity(profile, remote)?;
    let subscriptions = remote.subscriptions()?;
    for index in 0..journal.intents.len() {
        let intent = journal.intents[index].clone();
        if intent.profile_id != profile_id
            || matches!(
                intent.status,
                IntentStatus::Confirmed | IntentStatus::Canceled
            )
        {
            continue;
        }
        if intent.status == IntentStatus::Unknown {
            let hash = feed_hash(&intent.url)?;
            if let Some(subscription) = subscriptions
                .iter()
                .find(|subscription| feed_hash(&subscription.feed_url).ok().as_ref() == Some(&hash))
            {
                acknowledge(&mut journal, index, subscription.clone(), &local)?;
            }
            continue;
        }
        let current = api
            .subscriptions()?
            .into_iter()
            .find(|subscription| subscription.subscription_id == intent.local_id);
        if !current.is_some_and(|subscription| {
            feed_hash(&subscription.feed_url).ok() == feed_hash(&intent.url).ok()
        }) {
            journal.intents[index].status = IntentStatus::Canceled;
            journal.save(&local)?;
            continue;
        }
        journal.intents[index].status = IntentStatus::Unknown;
        journal.save(&local)?;
        match remote.ensure_subscription(&intent.url) {
            Ok(created) => acknowledge(&mut journal, index, created.subscription, &local)?,
            Err(error) => {
                if matches!(
                    error,
                    ApiError::Unauthorized | ApiError::Forbidden | ApiError::RateLimited
                ) {
                    journal.intents[index].status = IntentStatus::Ready;
                    journal.save(&local)?;
                }
                return Err(error);
            }
        }
    }
    Ok(())
}

fn acknowledge(
    journal: &mut Journal,
    index: usize,
    remote: Subscription,
    local: &Local,
) -> Result<(), ApiError> {
    let intent = &mut journal.intents[index];
    let profile_id = intent.profile_id.clone();
    let local_id = intent.local_id.clone();
    let hash = feed_hash(&intent.url)?;
    if feed_hash(&remote.feed_url)? != hash {
        return Err(ApiError::Response);
    }
    intent.status = IntentStatus::Confirmed;
    if !journal
        .links
        .iter()
        .any(|link| link.profile_id == profile_id && link.remote_id == remote.subscription_id)
    {
        journal.links.push(Link {
            profile_id,
            remote_id: remote.subscription_id.clone(),
            local_id,
            feed_hash: hash,
            remote,
        });
    }
    journal.save(local)
}

pub(crate) fn cancel_intent(api: &Api, id: &str) -> Result<(), ApiError> {
    let local = local_core(api)?;
    let _guard = local.sync_lock.lock().map_err(|_| ApiError::LocalData)?;
    let mut journal = Journal::load(&local)?;
    let intent = journal
        .intents
        .iter_mut()
        .find(|intent| intent.id == id)
        .ok_or(ApiError::Response)?;
    intent.status = IntentStatus::Canceled;
    journal.save(&local)
}

pub(crate) fn unsubscribe(api: &Api, subscription_id: &str) -> Result<(), ApiError> {
    let local = local_core(api)?;
    let _guard = local.sync_lock.lock().map_err(|_| ApiError::LocalData)?;
    let mut journal = Journal::load(&local)?;
    if let Some(subscription) = api
        .subscriptions()?
        .iter()
        .find(|subscription| subscription.subscription_id == subscription_id)
    {
        let hash = feed_hash(&subscription.feed_url)?;
        for source in journal
            .pending_imports
            .iter()
            .filter(|(_, pending_hash)| pending_hash == &hash)
        {
            journal.suppressions.insert(source.clone());
        }
        journal
            .pending_imports
            .retain(|(_, pending_hash)| pending_hash != &hash);
    }
    for link in journal
        .links
        .iter()
        .filter(|link| link.local_id == subscription_id)
    {
        journal
            .suppressions
            .insert((link.profile_id.clone(), link.feed_hash.clone()));
    }
    for intent in journal
        .intents
        .iter()
        .filter(|intent| intent.local_id == subscription_id)
    {
        journal
            .suppressions
            .insert((intent.profile_id.clone(), feed_hash(&intent.url)?));
    }
    for intent in journal
        .intents
        .iter_mut()
        .filter(|intent| intent.local_id == subscription_id && intent.status == IntentStatus::Ready)
    {
        intent.status = IntentStatus::Canceled;
    }
    journal.save(&local)?;
    api.write::<serde_json::Value>(
        Method::DELETE,
        &format!("api/v1/subscriptions/{subscription_id}"),
        &serde_json::Value::Null,
    )?;
    Ok(())
}

pub(crate) fn disconnect(api: &Api, profile_id: &str) -> Result<(), ApiError> {
    let local = local_core(api)?;
    let _guard = local.sync_lock.lock().map_err(|_| ApiError::LocalData)?;
    let mut journal = Journal::load(&local)?;
    journal.profiles.retain(|profile| profile.id != profile_id);
    journal
        .intents
        .retain(|intent| intent.profile_id != profile_id);
    journal.links.retain(|link| link.profile_id != profile_id);
    journal
        .pending_imports
        .retain(|(profile, _)| profile != profile_id);
    journal
        .suppressions
        .retain(|(profile, _)| profile != profile_id);
    journal.save(&local)
}
