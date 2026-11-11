use nsh_base::{alert_dialog, dialog, theme};
use nsh_core::prelude::*;
use zeroize::Zeroizing;

use crate::{
    store::Store,
    sync::{IntentStatus, Profile},
    ui,
};

#[derive(Clone, Copy)]
pub(crate) struct Visibility(pub(crate) Signal<bool>);

#[component]
pub(crate) fn Settings() -> impl IntoElement {
    let store = use_context::<Store>().expect("settings has a store");
    let visible = use_context::<Visibility>()
        .expect("settings has an owner")
        .0;
    let selected = signal(None::<String>);
    let form = signal(false);
    let address = signal(String::new());
    let login = signal(String::new());
    let password = signal(String::new());
    let profile = move || {
        store.read(|state| {
            selected
                .get()
                .as_ref()
                .and_then(|id| state.remotes.iter().find(|remote| &remote.profile.id == id))
                .or_else(|| state.remotes.first())
                .map(|remote| remote.profile.clone())
        })
    };
    let connected = move || {
        profile().is_some_and(|profile| {
            store.read(|state| {
                state
                    .remotes
                    .iter()
                    .any(|remote| remote.profile.id == profile.id && remote.api.is_some())
            })
        })
    };
    let close = move || {
        password.update(|password| {
            use zeroize::Zeroize;
            password.zeroize();
            password.clear();
        });
        visible.set(false);
    };
    let submit = move || {
        let secret = Zeroizing::new(password.get_untracked());
        password.update(|password| {
            use zeroize::Zeroize;
            password.zeroize();
            password.clear();
        });
        store.login(address.get_untracked(), login.get_untracked(), secret);
        form.set(false);
    };
    dialog().on_cancel(close).flex_1().bg(move || theme().background)
        .child(ui::toolbar().child(text(move || crate::i18n::tr("服务器与同步")).text_base().font_weight(FontWeight::SEMIBOLD).flex_1())
            .child(ui::command("完成", close)))
        .child(view().flex_row().flex_1().min_h_0()
            .child(view().w_64().min_w_0().bg(move || theme().sidebar).border_r_1().border_color(move || theme().border)
                .child(view().p_4().gap_2().child(ui::heading("本地资料库")).child(ui::caption("无需服务器 · 数据保存在此设备")))
                .child(virtual_list(move || store.read(|state| state.remotes.len()), move |at| {
                    let remote = move || store.read(|state| state.remotes.get(at.index()).cloned());
                    ui::row(move || { if let Some(remote) = remote() { selected.set(Some(remote.profile.id)); form.set(false); } })
                        .mx_2().my_1()
                        .selected(move || remote().zip(profile()).is_some_and(|(remote, profile)| remote.profile.id == profile.id))
                        .child(text(move || remote().map(|remote| remote.profile.label()).unwrap_or_default()).text_sm().truncate())
                        .child(ui::caption(move || remote().map(|remote| crate::i18n::tr(if remote.api.is_some() { "已连接 · 兼容模式" } else { "需要登录" })).unwrap_or_default()))
                }).keys(move || store.read(|state| state.remotes.iter().map(|remote| remote.profile.id.clone()).collect::<Vec<_>>()))
                    .estimated_item_size(px(64.0)).flex_1())
                .child(view().p_3().child(ui::command("添加服务器…", move || { address.set(String::new()); login.set(String::new()); form.set(true); })
                    .disabled(move || store.read(|state| state.connecting || state.sync_busy || state.sync_unavailable)))))
            .child(scroll().flex_1().min_w_0().min_h_0().child(view().p_6().gap_5()
                .child(appearance_settings(store))
                .child(when(form, move || {
                    view().gap_4()
                        .child(text(move || crate::i18n::tr("连接远端 Raindrop")).text_xl().font_weight(FontWeight::SEMIBOLD))
                        .child(ui::caption("登录后导入订阅与可见 AI 配置，不上传本地存量，不复制 API 密钥。"))
                        .child(ui::field("HTTPS 服务地址", address))
                        .child(ui::field("用户名或邮箱", login))
                        .child(ui::field("密码", password).masked(true).max_length(1024).on_submit(move |_: &str| {
                            if !password.with_untracked(String::is_empty) && !store.peek(|state| state.connecting || state.sync_busy) { submit(); }
                        }))
                        .child(view().flex_row().gap_2().justify_end()
                            .child(ui::command("取消", move || { password.set(String::new()); form.set(false); }))
                            .child(ui::primary("连接并导入", submit).disabled(move || address.with(String::is_empty) || login.with(String::is_empty) || password.with(String::is_empty) || store.read(|state| state.connecting || state.sync_busy))))
                }).otherwise(move || {
                    when(move || profile().is_some(), move || profile_details(store, profile, connected, selected, form, address, login))
                        .otherwise(|| view().gap_3().child(text(move || crate::i18n::tr("可选的远端连接")).text_xl().font_weight(FontWeight::SEMIBOLD))
                            .child(ui::caption("本地阅读不依赖远端。添加服务器后，可导入订阅和 AI 配置，或明确推送选中的本地订阅。")))
                }))
                .child(when(move || store.read(|state| state.connecting || state.sync_busy), || ui::caption("正在处理，请稍候…")))
                .child(when(move || store.read(|state| !state.error.is_empty()), move || text(move || store.read(|state| crate::i18n::tr(&state.error))).text_sm().text_color(move || theme().destructive)))
                .child(when(move || store.read(|state| !state.notice.is_empty()), move || ui::caption(move || store.read(|state| state.notice.localized())))))))
}

fn appearance_settings(store: Store) -> impl IntoElement {
    use crate::preferences::{Language, Theme};
    view()
        .gap_3()
        .child(ui::heading("外观与语言"))
        .child(ui::caption("仅影响此设备，自动保存。"))
        .child(ui::caption("语言"))
        .child(
            view().flex_row().gap_2().flex_wrap().children(
                [
                    (Language::Chinese, "简体中文"),
                    (Language::English, "English"),
                ]
                .map(move |(language, label)| {
                    ui::row(move || {
                        let mut preferences = store.peek(|state| state.preferences);
                        preferences.language = language;
                        store.set_preferences(preferences);
                    })
                    .selected(move || store.read(|state| state.preferences.language == language))
                    .disabled(move || {
                        store.read(|state| state.preferences_busy || state.preferences_unavailable)
                    })
                    .child(text(label).text_sm())
                }),
            ),
        )
        .child(ui::caption("主题"))
        .child(
            view().flex_row().gap_2().flex_wrap().children(
                [
                    (Theme::System, "跟随系统"),
                    (Theme::Light, "浅色"),
                    (Theme::Dark, "深色"),
                ]
                .map(move |(theme, label)| {
                    ui::row(move || {
                        let mut preferences = store.peek(|state| state.preferences);
                        preferences.theme = theme;
                        store.set_preferences(preferences);
                    })
                    .selected(move || store.read(|state| state.preferences.theme == theme))
                    .disabled(move || {
                        store.read(|state| state.preferences_busy || state.preferences_unavailable)
                    })
                    .child(text(move || crate::i18n::tr(label)).text_sm())
                }),
            ),
        )
        .child(when(
            move || store.read(|state| state.preferences_unavailable),
            || ui::caption("无法读取外观设置，已使用默认值；原有设置未被覆盖。"),
        ))
        .pb_5()
        .border_b_1()
        .border_color(move || theme().border)
}

fn profile_details(
    store: Store,
    profile: impl Fn() -> Option<Profile> + Copy + 'static,
    connected: impl Fn() -> bool + Copy + 'static,
    selected: Signal<Option<String>>,
    form: Signal<bool>,
    address: Signal<String>,
    login: Signal<String>,
) -> impl IntoElement {
    view().gap_5()
        .child(text(move || profile().map(|profile| profile.label()).unwrap_or_default()).text_xl().font_weight(FontWeight::SEMIBOLD))
        .child(ui::caption(move || profile().map(|profile| profile.address).unwrap_or_default()))
        .child(view().flex_row().gap_2().flex_wrap()
            .child(ui::command("拉取订阅与 AI 配置", move || { if let Some(profile) = profile() { store.import_remote(profile.id); } })
                .disabled(move || !connected() || store.read(|state| state.sync_busy || state.connecting)))
            .child(ui::command("重新登录", move || { if let Some(profile) = profile() { address.set(profile.address); login.set(profile.username); form.set(true); } })
                .disabled(move || store.read(|state| state.sync_busy || state.connecting)))
            .child(ui::command("移除连接…", move || {
                if let Some(profile) = profile() {
                    spawn(async move {
                        let result = alert_dialog().title(crate::i18n::tr("移除服务器连接？").to_string()).description(crate::i18n::tr("保留已导入本地的订阅；取消此连接尚未完成的推送任务。").to_string())
                            .cancel("cancel", crate::i18n::tr("取消").to_string()).destructive("remove", crate::i18n::tr("移除连接").to_string()).show().await;
                        if result.is_ok_and(|result| result.action.as_deref() == Some("remove")) { store.disconnect(profile.id); selected.set(None); }
                    });
                }
            }).disabled(move || store.read(|state| state.sync_busy || state.connecting))))
        .child(view().gap_2().child(ui::heading("同步范围"))
            .child(ui::caption("兼容导入：拉取新增订阅，关联相同 Feed；保留本地标题、分类和顺序。远端修改不会直接覆盖本地。"))
            .child(ui::caption("单向推送：只推送下方明确选中的新增订阅，不修改或删除远端已有订阅。"))
            .child(ui::caption("可靠双向：当前服务器未协商修订、幂等和删除记录协议，因此暂不可用。")))
        .child(ui::heading("远端 AI 配置"))
        .child(ui::caption("以下仅为远端引用，不含 API 密钥。尚未关联远端文章身份，不会自动执行 AI，也不会静默改用本地 Provider。"))
        .child(ui::caption(move || profile().map(|profile| {
            let enabled = profile.ai.config.as_ref().is_some_and(|config| config.is_enabled);
            format!("{} · AI: {}", profile.ai.plugin_state, crate::i18n::tr(if enabled { "已启用" } else { "未启用" }))
        }).unwrap_or_default()))
        .child(each(move || profile().map(|profile| profile.providers).unwrap_or_default(), |provider| provider.provider_id.clone(), move |provider, _| {
            view().gap_1().child(ui::heading(move || provider.with(|provider| provider.display_name.clone())))
                .child(ui::caption(move || provider.with(|provider| format!("{} · {} · {}", provider.kind, provider.model, crate::i18n::tr(if provider.is_enabled { "远端已启用" } else { "远端未启用" })))))
        }))
        .child(ui::heading("推送新增订阅"))
        .child(ui::caption("可离线保存发送意图。连接后点「发送待处理」；结果未知时只核对，不自动重发。"))
        .child(ui::command("发送待处理 / 核对结果", move || { if let Some(profile) = profile() { store.push_remote(profile.id); } })
            .disabled(move || !connected() || store.read(|state| state.sync_busy || state.connecting)))
        .child(virtual_list(move || store.read(|state| state.subscriptions.len()), move |at| {
            let subscription = move || store.read(|state| state.subscriptions.get(at.index()).cloned());
            view().flex_row().items_center().gap_3().px_1().py_1()
                .child(text(move || subscription().map(|subscription| subscription.title).unwrap_or_default()).text_sm().truncate().flex_1())
                .child(ui::command("加入待发送", move || {
                    if let (Some(profile), Some(subscription)) = (profile(), subscription()) { store.queue_push(profile.id, subscription.subscription_id); }
                }).disabled(move || store.read(|state| state.sync_busy || state.connecting)))
        }).keys(move || store.read(|state| state.subscriptions.iter().map(|subscription| subscription.subscription_id.clone()).collect::<Vec<_>>()))
            .h_48().estimated_item_size(px(40.0)))
        .child(each(move || store.read(|state| state.intents.iter().filter(|intent| profile().is_some_and(|profile| intent.profile_id == profile.id)).cloned().collect::<Vec<_>>()), |intent| intent.id.clone(), move |intent, _| {
            view().flex_row().gap_3().items_center()
                .child(ui::caption(move || intent.with(|intent| format!("{} · {}", intent.id.chars().take(8).collect::<String>(), crate::i18n::tr(intent.status.label())))).flex_1())
                .child(ui::command("取消此任务", move || store.cancel_intent(intent.with_untracked(|intent| intent.id.clone())))
                    .disabled(move || intent.with(|intent| matches!(intent.status, IntentStatus::Canceled | IntentStatus::Confirmed)) || store.read(|state| state.sync_busy)))
        }))
}
