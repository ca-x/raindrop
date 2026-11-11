use nsh_core::prelude::*;
use zeroize::Zeroizing;

use crate::api::{Api, ApiError, Entry, EntryPatch, Filter, Query, Subscription, background};

#[derive(Clone)]
pub(crate) struct Remote {
    pub(crate) profile: crate::sync::Profile,
    pub(crate) api: Option<Api>,
}

#[derive(Clone, Default)]
pub(crate) struct State {
    pub(crate) preferences: crate::preferences::Preferences,
    pub(crate) preferences_busy: bool,
    pub(crate) preferences_unavailable: bool,
    pub(crate) api: Option<Api>,
    pub(crate) username: SharedString,
    pub(crate) subscriptions: Vec<Subscription>,
    pub(crate) entries: Vec<Entry>,
    pub(crate) query: Query,
    pub(crate) selected: Option<String>,
    pub(crate) detail: Option<Entry>,
    pub(crate) next_cursor: Option<String>,
    pub(crate) error: SharedString,
    pub(crate) connecting: bool,
    pub(crate) loading: bool,
    pub(crate) loading_detail: bool,
    pub(crate) writing: bool,
    pub(crate) notice: Notice,
    pub(crate) remotes: Vec<Remote>,
    pub(crate) intents: Vec<crate::sync::Intent>,
    pub(crate) sync_busy: bool,
    pub(crate) sync_unavailable: bool,
    session_generation: u64,
    list_generation: u64,
    detail_generation: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct Store(Signal<State>, Owner);

#[derive(Clone, Default)]
pub(crate) enum Notice {
    #[default]
    Empty,
    Message(String),
    Imported([usize; 4]),
}

impl From<&str> for Notice {
    fn from(message: &str) -> Self {
        Self::Message(message.to_owned())
    }
}

impl Notice {
    pub(crate) fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    pub(crate) fn localized(&self) -> SharedString {
        match self {
            Self::Empty => "".into(),
            Self::Message(message) => crate::i18n::tr(message),
            Self::Imported(counts) => {
                let mut message = crate::i18n::tr("已导入 {} 个订阅，关联 {} 个；{} 个远端修改保留待核对，{} 个已取消的来源未重新添加。").to_string();
                for count in counts {
                    message = message.replacen("{}", &count.to_string(), 1);
                }
                message.into()
            }
        }
    }
}

impl Store {
    pub(crate) fn new() -> Self {
        Self(
            signal_with(State::default(), SignalEquals::Never),
            Owner::current(),
        )
    }

    pub(crate) fn spawn(self, work: impl std::future::Future<Output = ()> + 'static) {
        self.1.run(|| spawn(work));
    }

    #[cfg(test)]
    pub(crate) fn with_api(api: Api) -> Self {
        let store = Self::new();
        let journal = crate::sync::profiles(&api);
        let preferences = api
            .local_core()
            .ok_or(ApiError::LocalData)
            .and_then(|local| crate::preferences::Preferences::load(&local));
        store.attach_local(api, journal, preferences);
        store
    }

    fn attach_local(
        self,
        api: Api,
        journal: Result<crate::sync::Journal, ApiError>,
        preferences: Result<crate::preferences::Preferences, ApiError>,
    ) {
        self.0.update(|state| {
            match preferences {
                Ok(preferences) => state.preferences = preferences,
                Err(_) => {
                    state.preferences_unavailable = true;
                    state.error = "无法读取外观设置，已使用默认值；原有设置未被覆盖。".into();
                }
            }
            state.api = Some(api);
            state.username = "本地资料库".into();
            match journal {
                Ok(journal) => {
                    state.remotes = journal.profiles.into_iter().map(|profile| Remote { profile, api: None }).collect();
                    state.intents = journal.intents;
                }
                Err(_) => {
                    state.sync_unavailable = true;
                    state.notice = "同步记录无法读取，已禁用远端操作；本地阅读仍可使用。请保留数据库以便恢复。".into();
                }
            }
        });
        set_color_scheme(self.peek(|state| state.preferences.theme.appearance()));
    }

    pub(crate) fn set_preferences(self, preferences: crate::preferences::Preferences) {
        if self.peek(|state| state.preferences_busy || state.preferences_unavailable) {
            return;
        }
        let Some(local) = self.peek(|state| state.api.as_ref().and_then(Api::local_core)) else {
            return;
        };
        self.0.update(|state| state.preferences_busy = true);
        self.spawn(async move {
            let result = background(move || preferences.save(&local)).await;
            self.0.update(|state| {
                state.preferences_busy = false;
                match result {
                    Ok(()) => state.preferences = preferences,
                    Err(_) => state.error = "无法保存外观设置，请检查磁盘空间后重试。".into(),
                }
            });
            if result.is_ok() {
                set_color_scheme(preferences.theme.appearance());
            }
        });
    }

    pub(crate) fn remove_current(self) {
        if self.peek(|state| state.writing || state.sync_busy || state.connecting) {
            return;
        }
        let Some((api, subscription_id)) = self.peek(|state| {
            Some((
                state.api.clone()?,
                state
                    .subscriptions
                    .iter()
                    .find(|subscription| {
                        Some(&subscription.feed_id) == state.query.feed_id.as_ref()
                    })?
                    .subscription_id
                    .clone(),
            ))
        }) else {
            return;
        };
        self.0.update(|state| state.writing = true);
        self.spawn(async move {
            let result = background(move || {
                crate::sync::unsubscribe(&api, &subscription_id)?;
                crate::sync::profiles(&api)
            })
            .await;
            self.0.update(|state| state.writing = false);
            match result {
                Ok(journal) => {
                    self.update_journal(journal);
                    self.filter(Filter::All, None);
                }
                Err(error) => self.fail(error),
            }
        });
    }

    pub(crate) fn read<T>(self, read: impl FnOnce(&State) -> T) -> T {
        self.0.with(read)
    }

    pub(crate) fn peek<T>(self, read: impl FnOnce(&State) -> T) -> T {
        self.0.with_untracked(read)
    }

    pub(crate) fn start_local(self) {
        if self.peek(|state| state.connecting || state.api.is_some()) {
            return;
        }
        self.0.update(|state| {
            state.connecting = true;
            state.error = "".into();
        });
        self.spawn(async move {
            let result = background(move || {
                let api = Api::local(crate::local::data_dir()?)?;
                let journal = crate::sync::profiles(&api);
                let preferences = api
                    .local_core()
                    .ok_or(ApiError::LocalData)
                    .and_then(|local| crate::preferences::Preferences::load(&local));
                Ok((api, journal, preferences))
            })
            .await;
            self.0.update(|state| state.connecting = false);
            match result {
                Ok((api, journal, preferences)) => {
                    self.attach_local(api, journal, preferences);
                    self.reload();
                }
                Err(error) => self.fail(error),
            }
        });
    }

    pub(crate) fn create_subscription(self, address: String) {
        let Some(api) = self.peek(|state| (!state.writing).then(|| state.api.clone()).flatten())
        else {
            return;
        };
        self.0.update(|state| {
            state.writing = true;
            state.error = "".into();
        });
        self.spawn(async move {
            let result = background(move || api.subscribe(&address)).await;
            self.0.update(|state| state.writing = false);
            match result {
                Ok(subscription) => {
                    self.0
                        .update(|state| state.notice = "已添加订阅，正在后台获取文章。".into());
                    self.filter(Filter::All, Some(subscription.feed_id));
                }
                Err(error) => self.fail(error),
            }
        });
    }

    pub(crate) fn refresh_current(self) {
        let Some((api, subscription)) = self.peek(|state| {
            let api = state.api.clone()?;
            let feed = state.query.feed_id.as_ref()?;
            let subscription = state
                .subscriptions
                .iter()
                .find(|subscription| &subscription.feed_id == feed)?;
            (!state.writing).then(|| (api, subscription.subscription_id.clone()))
        }) else {
            return;
        };
        self.0.update(|state| {
            state.writing = true;
            state.error = "".into();
        });
        self.spawn(async move {
            let result = background(move || api.refresh(&subscription)).await;
            self.0.update(|state| state.writing = false);
            match result {
                Ok(()) => {
                    self.0.update(|state| {
                        state.notice = "已安排抓取；文章就绪后重新载入列表。".into()
                    });
                    self.reload();
                }
                Err(error) => self.fail(error),
            }
        });
    }

    pub(crate) fn login(self, address: String, login: String, password: Zeroizing<String>) {
        if self.peek(|state| state.sync_unavailable) {
            self.remote_fail(ApiError::LocalData);
            return;
        }
        if self.peek(|state| state.connecting || state.sync_busy) {
            return;
        }
        let Some(local_api) = self.peek(|state| state.api.clone()) else {
            return;
        };
        self.0.update(|state| {
            state.connecting = true;
            state.error = "".into();
        });
        let generation = self.peek(|state| state.session_generation);
        self.spawn(async move {
            let result = background(move || {
                let (api, username) = Api::login(&address, &login, password)?;
                let profile = crate::sync::connect(&local_api, &api, username)?;
                Ok((api, profile))
            })
            .await;
            if self.peek(|state| state.session_generation) != generation {
                return;
            }
            self.0.update(|state| state.connecting = false);
            match result {
                Ok((api, profile)) => {
                    let id = profile.id.clone();
                    self.0.update(|state| {
                        if let Some(existing) = state
                            .remotes
                            .iter_mut()
                            .find(|remote| remote.profile.id == profile.id)
                        {
                            existing.api = Some(api);
                            existing.profile = profile;
                        } else {
                            state.remotes.push(Remote {
                                profile,
                                api: Some(api),
                            });
                        }
                    });
                    self.import_remote(id);
                }
                Err(error) => self.remote_fail(error),
            }
        });
    }

    pub(crate) fn import_remote(self, id: String) {
        self.remote_action(id, |local, remote, id| {
            let report = crate::sync::import(local, remote, id)?;
            Ok(Notice::Imported([
                report.added,
                report.linked,
                report.changed,
                report.suppressed,
            ]))
        });
    }

    pub(crate) fn push_remote(self, id: String) {
        self.remote_action(id, |local, remote, id| {
            crate::sync::push_pending(local, remote, id)?;
            Ok("已处理待发送任务；结果未知的任务不会自动重发。".into())
        });
    }

    fn remote_action(
        self,
        id: String,
        action: impl FnOnce(&Api, &Api, &str) -> Result<Notice, ApiError> + Send + 'static,
    ) {
        if self.peek(|state| state.sync_unavailable) {
            self.remote_fail(ApiError::LocalData);
            return;
        }
        if self.peek(|state| state.sync_busy || state.connecting) {
            return;
        }
        let Some((local, remote)) = self.peek(|state| {
            Some((
                state.api.clone()?,
                state
                    .remotes
                    .iter()
                    .find(|remote| remote.profile.id == id)?
                    .api
                    .clone()?,
            ))
        }) else {
            self.remote_fail(ApiError::Unauthorized);
            return;
        };
        self.0.update(|state| {
            state.sync_busy = true;
            state.error = "".into();
        });
        self.spawn(async move {
            let result = background(move || {
                let result = action(&local, &remote, &id);
                let journal = crate::sync::profiles(&local)?;
                Ok((result, journal))
            })
            .await;
            self.0.update(|state| state.sync_busy = false);
            match result {
                Ok((result, journal)) => {
                    self.update_journal(journal);
                    match result {
                        Ok(notice) => self.0.update(|state| state.notice = notice),
                        Err(error) => self.remote_fail(error),
                    }
                    self.reload();
                }
                Err(error) => self.remote_fail(error),
            }
        });
    }

    pub(crate) fn queue_push(self, profile_id: String, subscription_id: String) {
        self.journal_action(move |local| {
            crate::sync::queue_push(local, &profile_id, &subscription_id)
        });
    }

    pub(crate) fn cancel_intent(self, id: String) {
        self.journal_action(move |local| crate::sync::cancel_intent(local, &id));
    }

    pub(crate) fn disconnect(self, id: String) {
        let remote = self.peek(|state| {
            state
                .remotes
                .iter()
                .find(|remote| remote.profile.id == id)
                .and_then(|remote| remote.api.clone())
        });
        self.journal_action(move |local| {
            crate::sync::disconnect(local, &id)?;
            if let Some(remote) = remote {
                let _ = remote.logout();
            }
            Ok(())
        });
    }

    fn journal_action(self, action: impl FnOnce(&Api) -> Result<(), ApiError> + Send + 'static) {
        if self.peek(|state| state.sync_unavailable) {
            self.remote_fail(ApiError::LocalData);
            return;
        }
        if self.peek(|state| state.sync_busy || state.connecting) {
            return;
        }
        let Some(local) = self.peek(|state| state.api.clone()) else {
            return;
        };
        self.0.update(|state| {
            state.sync_busy = true;
            state.error = "".into();
        });
        self.spawn(async move {
            let result = background(move || {
                action(&local)?;
                crate::sync::profiles(&local)
            })
            .await;
            self.0.update(|state| state.sync_busy = false);
            match result {
                Ok(journal) => self.update_journal(journal),
                Err(error) => self.remote_fail(error),
            }
        });
    }

    fn update_journal(self, journal: crate::sync::Journal) {
        self.0.update(|state| {
            state.remotes = journal
                .profiles
                .into_iter()
                .map(|profile| {
                    let api = state
                        .remotes
                        .iter()
                        .find(|remote| remote.profile.id == profile.id)
                        .and_then(|remote| remote.api.clone());
                    Remote { profile, api }
                })
                .collect();
            state.intents = journal.intents;
        });
    }

    fn remote_fail(self, error: ApiError) {
        self.0.update(|state| state.error = error.message().into());
    }

    pub(crate) fn reload(self) {
        self.load(false);
    }

    pub(crate) fn load_more(self) {
        if self.peek(|state| !state.loading && state.next_cursor.is_some()) {
            self.load(true);
        }
    }

    fn load(self, append: bool) {
        let Some(api) = self.peek(|state| state.api.clone()) else {
            return;
        };
        self.0.update(|state| {
            state.loading = true;
            state.error = "".into();
            state.list_generation += 1;
        });
        let (generation, session, query, cursor) = self.peek(|state| {
            (
                state.list_generation,
                state.session_generation,
                state.query.clone(),
                append.then(|| state.next_cursor.clone()).flatten(),
            )
        });
        self.spawn(async move {
            let result = background(move || {
                let page = api.entries(&query, cursor.as_deref())?;
                let subscriptions = if append {
                    None
                } else {
                    Some(api.subscriptions()?)
                };
                Ok((page, subscriptions))
            })
            .await;
            if !self.peek(|state| {
                state.list_generation == generation && state.session_generation == session
            }) {
                return;
            }
            self.0.update(|state| state.loading = false);
            match result {
                Ok((page, subscriptions)) => self.0.update(|state| {
                    if append {
                        for entry in page.items {
                            if !state
                                .entries
                                .iter()
                                .any(|existing| existing.entry_id == entry.entry_id)
                            {
                                state.entries.push(entry);
                            }
                        }
                    } else {
                        state.entries = page.items;
                    }
                    state.next_cursor = page.next_cursor;
                    if let Some(subscriptions) = subscriptions {
                        state.subscriptions = subscriptions;
                    }
                }),
                Err(error) => self.fail(error),
            }
        });
    }

    pub(crate) fn filter(self, filter: Filter, feed_id: Option<String>) {
        self.0.update(|state| {
            state.query.filter = filter;
            state.query.feed_id = feed_id;
        });
        self.query_changed();
    }

    pub(crate) fn search(self, search: String) {
        self.0.update(|state| state.query.search = search);
        self.query_changed();
    }

    fn query_changed(self) {
        self.0.update(|state| {
            state.entries.clear();
            state.next_cursor = None;
            state.selected = None;
            state.detail = None;
            state.loading_detail = false;
            state.detail_generation += 1;
        });
        self.reload();
    }

    pub(crate) fn select(self, id: String) {
        let Some(api) = self.peek(|state| state.api.clone()) else {
            return;
        };
        self.0.update(|state| {
            state.selected = Some(id.clone());
            state.detail = None;
            state.loading_detail = true;
            state.error = "".into();
            state.detail_generation += 1;
        });
        let (generation, session) =
            self.peek(|state| (state.detail_generation, state.session_generation));
        self.spawn(async move {
            let result = background(move || api.entry(&id)).await;
            if !self.peek(|state| {
                state.detail_generation == generation && state.session_generation == session
            }) {
                return;
            }
            self.0.update(|state| state.loading_detail = false);
            match result {
                Ok(detail) => self.0.update(|state| state.detail = Some(detail)),
                Err(error) => self.fail(error),
            }
        });
    }

    pub(crate) fn toggle_read(self) {
        if let Some(read) = self.peek(|state| state.detail.as_ref().map(|entry| entry.is_read)) {
            self.patch(EntryPatch {
                is_read: Some(!read),
                is_starred: None,
            });
        }
    }

    pub(crate) fn toggle_star(self) {
        if let Some(starred) =
            self.peek(|state| state.detail.as_ref().map(|entry| entry.is_starred))
        {
            self.patch(EntryPatch {
                is_read: None,
                is_starred: Some(!starred),
            });
        }
    }

    fn patch(self, patch: EntryPatch) {
        let Some((api, entry, session)) = self.peek(|state| {
            (!state.writing)
                .then(|| {
                    Some((
                        state.api.clone()?,
                        state.detail.clone()?,
                        state.session_generation,
                    ))
                })
                .flatten()
        }) else {
            return;
        };
        self.0.update(|state| {
            state.writing = true;
            state.error = "".into();
        });
        self.spawn(async move {
            let id = entry.entry_id.clone();
            let result = background(move || api.patch(&id, patch)).await;
            if self.peek(|state| state.session_generation) != session {
                return;
            }
            self.0.update(|state| state.writing = false);
            match result {
                Ok(updated) if updated.entry_id == entry.entry_id => self.0.update(|state| {
                    let unread_delta = i64::from(entry.is_read) - i64::from(updated.is_read);
                    for subscription in &mut state.subscriptions {
                        if subscription.feed_id == entry.feed_id {
                            subscription.unread_count =
                                (subscription.unread_count + unread_delta).max(0);
                        }
                    }
                    for current in &mut state.entries {
                        if current.entry_id == updated.entry_id {
                            current.is_read = updated.is_read;
                            current.is_starred = updated.is_starred;
                        }
                    }
                    if let Some(detail) = &mut state.detail
                        && detail.entry_id == updated.entry_id
                    {
                        detail.is_read = updated.is_read;
                        detail.is_starred = updated.is_starred;
                    }
                    state.entries.retain(|entry| match state.query.filter {
                        Filter::All => true,
                        Filter::Unread => !entry.is_read,
                        Filter::Starred => entry.is_starred,
                    });
                }),
                Ok(_) => self.fail(ApiError::Response),
                Err(error) => self.fail(error),
            }
        });
    }

    pub(crate) fn move_selection(self, offset: isize) {
        let id = self.peek(|state| {
            let index = state
                .selected
                .as_ref()
                .and_then(|id| state.entries.iter().position(|entry| &entry.entry_id == id));
            let next = match index {
                Some(index) => index
                    .saturating_add_signed(offset)
                    .min(state.entries.len().saturating_sub(1)),
                None => 0,
            };
            state.entries.get(next).map(|entry| entry.entry_id.clone())
        });
        if let Some(id) = id {
            self.select(id);
        }
    }

    pub(crate) fn clear_selection(self) {
        self.0.update(|state| {
            state.selected = None;
            state.detail = None;
            state.detail_generation += 1;
            state.loading_detail = false;
        });
    }

    fn reset(self) {
        self.0.update(|state| {
            *state = State {
                session_generation: state.session_generation + 1,
                ..State::default()
            };
        });
    }

    fn fail(self, error: ApiError) {
        if error == ApiError::Unauthorized && self.peek(|state| state.api.is_some()) {
            self.reset();
        }
        self.0.update(|state| state.error = error.message().into());
    }
}
