//! Optional capacity hints. Catalog data never supplies endpoints, credentials or routing.

use std::{sync::Arc, time::Duration};

use models_dev_rs::{CacheMetadata, Catalog, Client, Refresh};
use serde::Serialize;
use tokio::{sync::Mutex, time::Instant};

const REFRESH_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const RETRY_INTERVAL: Duration = Duration::from_secs(10 * 60);

struct Cache {
    catalog: Option<Arc<Catalog>>,
    metadata: Option<CacheMetadata>,
    refresh_after: Option<Instant>,
}

// Shared across users, containing public metadata only. A failed refresh keeps the last good data.
static CACHE: Mutex<Cache> = Mutex::const_new(Cache {
    catalog: None,
    metadata: None,
    refresh_after: None,
});

/// Fetch only the fixed public models.dev endpoint. No Provider URL or credential enters this client.
pub async fn model_catalog() -> Option<Arc<Catalog>> {
    let mut cache = CACHE.lock().await;
    if cache.refresh_after.is_some_and(|at| Instant::now() < at) {
        return cache.catalog.clone();
    }
    // Set before awaiting so cancelled and unsuccessful lookups also have a bounded retry rate.
    cache.refresh_after = Some(Instant::now() + RETRY_INTERVAL);
    let client = match Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(5))
        .build()
    {
        Ok(client) => client,
        Err(_) => return cache.catalog.clone(),
    };
    let result = match &cache.metadata {
        Some(metadata) => client.refresh(metadata).await,
        None => client.fetch().await.map(Refresh::Modified),
    };
    match result {
        Ok(Refresh::Modified(download)) => {
            cache.catalog = Some(Arc::new(download.catalog));
            cache.metadata = Some(download.metadata);
            cache.refresh_after = Some(Instant::now() + REFRESH_INTERVAL);
        }
        Ok(Refresh::NotModified(metadata)) => {
            cache.metadata = Some(metadata);
            cache.refresh_after = Some(Instant::now() + REFRESH_INTERVAL);
        }
        Err(_) => tracing::debug!("Optional model capacity catalog is unavailable"),
    }
    cache.catalog.clone()
}

/// Discovery has its own short deadline; it may use a warm cache without waiting on a download.
pub fn cached_model_catalog() -> Option<Arc<Catalog>> {
    CACHE.try_lock().ok()?.catalog.clone()
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelLimits {
    pub context_window: Option<u32>,
    pub max_input_tokens: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_kind: Option<&'static str>,
}

impl ModelLimits {
    /// An adapter's smaller declared capacity always wins over catalog metadata.
    #[must_use]
    pub fn merge(self, other: Self) -> Self {
        Self {
            context_window: minimum_known(self.context_window, other.context_window),
            max_input_tokens: minimum_known(self.max_input_tokens, other.max_input_tokens),
            max_output_tokens: minimum_known(self.max_output_tokens, other.max_output_tokens),
            source: "COMBINED",
            matched_model: self.matched_model.or(other.matched_model),
            match_kind: self.match_kind.or(other.match_kind),
        }
    }
}

fn minimum_known(left: Option<u32>, right: Option<u32>) -> Option<u32> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (left, right) => left.or(right),
    }
}

/// Prefer exact deployments, then unambiguous aliases. Aliases never change the requested model.
#[must_use]
pub fn catalog_model_limits(catalog: &Catalog, model: &str, endpoint: &str) -> Option<ModelLimits> {
    let model = model.strip_prefix("models/").unwrap_or(model);
    let official_provider =
        url::Url::parse(endpoint)
            .ok()
            .and_then(|url| match url.host_str()? {
                "api.openai.com" => Some("openai"),
                "api.anthropic.com" => Some("anthropic"),
                "generativelanguage.googleapis.com" => Some("google"),
                _ => None,
            });
    let exact = official_provider
        .and_then(|provider| catalog.model(provider, model))
        .or_else(|| {
            let (provider, id) = model.split_once('/')?;
            catalog.model(provider, id)
        });
    if let Some(model) = exact {
        return capacities(model);
    }
    let all_models = || {
        catalog
            .providers
            .values()
            .flat_map(|provider| provider.models.values())
    };
    let mut exact = all_models()
        .filter(|candidate| candidate.id == model)
        .peekable();
    if exact.peek().is_some() {
        return consistent_capacities(exact);
    }
    let key = alias_key(model)?;
    let aliases = all_models().filter(|candidate| alias_key(&candidate.id).as_ref() == Some(&key));
    let mut limits = consistent_capacities(aliases)?;
    limits.match_kind = Some("ALIAS");
    Some(limits)
}

// Normalize separators and a gateway namespace, never delete version digits, sizes or variant words.
fn alias_key(model: &str) -> Option<String> {
    let leaf = model.rsplit('/').next()?;
    if !leaf.is_ascii() {
        return None;
    }
    let key: String = leaf
        .chars()
        .filter(|character| !matches!(character, '.' | '-' | '_' | ':'))
        .map(|character| character.to_ascii_lowercase())
        .collect();
    (!key.is_empty()).then_some(key)
}

fn consistent_capacities<'model>(
    mut matches: impl Iterator<Item = &'model models_dev_rs::Model>,
) -> Option<ModelLimits> {
    let first = capacities(matches.next()?)?;
    for model in matches {
        let next = capacities(model)?;
        if first.context_window != next.context_window
            || first.max_input_tokens != next.max_input_tokens
            || first.max_output_tokens != next.max_output_tokens
        {
            return None;
        }
    }
    Some(first)
}

fn capacities(model: &models_dev_rs::Model) -> Option<ModelLimits> {
    let limits = ModelLimits {
        context_window: positive(model.limit.context),
        max_input_tokens: model.limit.input.and_then(positive),
        max_output_tokens: positive(model.limit.output),
        source: "MODELS_DEV",
        matched_model: Some(model.id.clone()),
        match_kind: Some("EXACT"),
    };
    (limits.context_window.is_some() || limits.max_output_tokens.is_some()).then_some(limits)
}

/// Preserve the service's capacity declarations; zero and malformed values are unknown.
#[must_use]
pub fn service_model_limits(value: &serde_json::Value) -> Option<ModelLimits> {
    let read = |fields: &[&str]| {
        fields
            .iter()
            .filter_map(|field| value.pointer(field)?.as_u64().and_then(positive))
            .min()
    };
    let limits = ModelLimits {
        context_window: read(&[
            "/context_length",
            "/context_window",
            "/limit/context",
            "/top_provider/context_length",
        ]),
        max_input_tokens: read(&["/max_input_tokens", "/inputTokenLimit", "/limit/input"]),
        max_output_tokens: read(&[
            "/max_output_tokens",
            "/outputTokenLimit",
            "/limit/output",
            "/top_provider/max_completion_tokens",
        ]),
        source: "PROVIDER",
        matched_model: None,
        match_kind: None,
    };
    (limits.context_window.is_some()
        || limits.max_input_tokens.is_some()
        || limits.max_output_tokens.is_some())
    .then_some(limits)
}

fn positive(value: u64) -> Option<u32> {
    u32::try_from(value).ok().filter(|value| *value > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Catalog {
        let model = |id, output| json!({"id":id,"name":id,"modalities":{"input":["text"],"output":["text"]},"limit":{"context":131072,"output":output}});
        let provider = |id, output| json!({"id":id,"name":id,"env":[],"npm":"adapter","doc":"https://example.com/","models":{"gpt-oss:120b":model("gpt-oss:120b",output)}});
        Catalog::from_slice(&serde_json::to_vec(&json!({"ollama-cloud":provider("ollama-cloud",32768),"other":provider("other",65536)})).unwrap()).unwrap()
    }

    #[test]
    fn qualified_ids_use_their_deployment_and_ambiguous_aliases_stay_unknown() {
        let catalog = fixture();
        let limits = catalog_model_limits(
            &catalog,
            "ollama-cloud/gpt-oss:120b",
            "https://gateway.example/",
        )
        .unwrap();
        assert_eq!(limits.context_window, Some(131072));
        assert_eq!(limits.max_output_tokens, Some(32768));
        assert!(
            catalog_model_limits(&catalog, "gpt-oss:120b", "https://gateway.example/").is_none()
        );
        assert!(catalog_model_limits(&catalog, "unknown", "https://gateway.example/").is_none());
    }

    #[test]
    fn smaller_service_limits_win_and_missing_values_stay_unknown() {
        let catalog = fixture();
        let limits = catalog_model_limits(&catalog, "other/gpt-oss:120b", "").unwrap();
        let service =
            service_model_limits(&json!({"max_output_tokens":32768,"context_window":128000}))
                .unwrap();
        let combined = limits.merge(service);
        assert_eq!(combined.max_output_tokens, Some(32768));
        assert_eq!(combined.context_window, Some(128000));
        assert_eq!(combined.max_input_tokens, None);
        assert_eq!(combined.source, "COMBINED");
        assert!(
            service_model_limits(&json!({"outputTokenLimit":0,"max_input_tokens":-1})).is_none()
        );
        assert_eq!(
            service_model_limits(&json!({"outputTokenLimit":65536}))
                .unwrap()
                .max_output_tokens,
            Some(65536)
        );
    }

    #[test]
    fn gateway_aliases_normalize_separators_without_confusing_model_variants() {
        let mut catalog = fixture();
        let provider = catalog.providers.get_mut("other").unwrap();
        let mut model = provider.models["gpt-oss:120b"].clone();
        model.id = "deepseek-41-flash".to_owned();
        provider.models.insert(model.id.clone(), model);
        let found = catalog_model_limits(&catalog, "auto/deepseek-4.1-flash", "")
            .expect("unambiguous alias");
        assert_eq!(found.max_output_tokens, Some(65536));
        assert_eq!(found.match_kind, Some("ALIAS"));
        assert_eq!(found.matched_model.as_deref(), Some("deepseek-41-flash"));
        for name in [
            "auto/deepseek-4.1-lite",
            "auto/deepseek-4.1-flash-preview",
            "auto/deepseek-4.2-flash",
            "auto/deepseek-4.1-flash-8b",
        ] {
            assert!(catalog_model_limits(&catalog, name, "").is_none(), "{name}");
        }
        let mut conflicting = catalog.providers["other"].models["deepseek-41-flash"].clone();
        conflicting.limit.output = 32768;
        catalog
            .providers
            .get_mut("ollama-cloud")
            .unwrap()
            .models
            .insert(conflicting.id.clone(), conflicting);
        assert!(catalog_model_limits(&catalog, "auto/deepseek-4.1-flash", "").is_none());
        assert_eq!(
            catalog_model_limits(&catalog, "other/deepseek-41-flash", "")
                .unwrap()
                .max_output_tokens,
            Some(65536)
        );
    }
}
