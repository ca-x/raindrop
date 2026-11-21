use crate::{
    api::{Api, background},
    speech::{Config, Status, VOICES},
    store::Store,
    ui,
};
use nsh_base::theme;
use nsh_core::{element::ImageSrc, prelude::*};
use std::time::Duration;

fn playback_status(store: Store) -> Signal<Status> {
    let status = signal(Status::Idle);
    set_interval(Duration::from_millis(80), move || {
        status.set(store.peek(|state| state.speech.status()));
    });
    status
}

const PLAY_ICON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m8 5 11 7-11 7z"/></svg>"#;
const PAUSE_ICON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 5v14M16 5v14"/></svg>"#;
const STOP_ICON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>"#;

pub(crate) fn controls(store: Store) -> impl IntoElement {
    let status = playback_status(store);
    cleanup(move || store.peek(|state| state.speech.clone()).stop());
    view()
        .flex_row()
        .gap_2()
        .items_center()
        .child(
            ui::icon_command(
                move || {
                    crate::i18n::tr(match status.get() {
                        Status::Playing | Status::Generating => "暂停朗读",
                        Status::Paused => "继续朗读",
                        _ => "朗读文章",
                    })
                },
                move || {
                    ImageSrc::svg_markup(
                        if matches!(status.get(), Status::Playing | Status::Generating) {
                            PAUSE_ICON
                        } else {
                            PLAY_ICON
                        },
                    )
                },
                move || {
                    let playback = store.peek(|state| state.speech.clone());
                    match playback.status() {
                        Status::Playing | Status::Generating | Status::Paused => {
                            playback.toggle_pause()
                        }
                        _ => {
                            if let Some((api, id)) = store.peek(|state| {
                                state
                                    .api
                                    .clone()
                                    .zip(state.detail.as_ref().map(|entry| entry.entry_id.clone()))
                            }) {
                                playback.article(api, id);
                            }
                        }
                    }
                },
            )
            .disabled(move || store.read(|state| state.detail.is_none())),
        )
        .child(
            ui::icon_command("停止朗读", ImageSrc::svg_markup(STOP_ICON), move || {
                store.peek(|state| state.speech.clone()).stop()
            })
            .disabled(move || matches!(status.get(), Status::Idle | Status::Failed(_))),
        )
}

pub(crate) fn article_feedback(store: Store) -> impl IntoElement {
    let status = playback_status(store);
    feedback(status)
}

fn feedback(status: Signal<Status>) -> impl IntoElement {
    view()
        .child(when(
            move || status.get() == Status::Generating,
            || ui::caption("正在生成语音…"),
        ))
        .child(when(
            move || matches!(status.get(), Status::Failed(_)),
            move || {
                text(move || {
                    crate::i18n::tr(if let Status::Failed(message) = status.get() {
                        message
                    } else {
                        ""
                    })
                })
                .text_sm()
                .text_color(move || theme().destructive)
            },
        ))
}

fn labeled(label: &'static str, value: Signal<String>, busy: Signal<bool>) -> impl IntoElement {
    ui::form_field(label, ui::field(label, value).disabled(busy))
}

#[component]
pub(crate) fn SpeechSettings() -> impl IntoElement {
    let store = use_context::<Store>().expect("speech settings has a store");
    let target = signal(String::new());
    let config = signal(None::<Config>);
    let endpoint = signal(String::new());
    let model = signal(String::new());
    let voice = signal(String::new());
    let custom = signal(false);
    let speed = signal(String::new());
    let enabled = signal(false);
    let key = signal(String::new());
    let clear = signal(false);
    let preview_text = signal("Raindrop speech preview.".to_owned());
    let busy = signal(false);
    let error = signal(String::new());
    let notice = signal(false);
    let epoch = signal(0_u64);
    let reload = signal(0_u64);
    let status = playback_status(store);
    let api = move || -> Option<Api> {
        let id = target.get_untracked();
        store.peek(|state| {
            if id.is_empty() {
                state.api.clone()
            } else {
                state
                    .remotes
                    .iter()
                    .find(|remote| remote.profile.id == id)
                    .and_then(|remote| remote.api.clone())
            }
        })
    };
    let wipe_key = move || {
        key.update(|key| {
            use zeroize::Zeroize;
            key.zeroize();
            key.clear();
        })
    };
    cleanup(move || {
        wipe_key();
        store.peek(|state| state.speech.clone()).stop();
    });
    effect(move || {
        let _ = target.get();
        let _ = reload.get();
        epoch.update(|value| *value += 1);
        let generation = epoch.get_untracked();
        store.peek(|state| state.speech.clone()).stop();
        wipe_key();
        clear.set(false);
        config.set(None);
        error.set(String::new());
        notice.set(false);
        let Some(api) = api() else {
            busy.set(false);
            error.set("登录信息无效或会话已过期，请重新登录。".into());
            return;
        };
        busy.set(true);
        spawn(async move {
            let result = background(move || api.read::<Config>("api/v1/plugins/tts")).await;
            if epoch.get_untracked() != generation {
                return;
            }
            busy.set(false);
            match result {
                Ok(value) => {
                    endpoint.set(value.base_url.clone());
                    model.set(value.model.clone());
                    voice.set(value.voice.clone());
                    custom.set(!VOICES.iter().any(|(id, _)| *id == value.voice));
                    speed.set(value.speed.to_string());
                    enabled.set(value.is_enabled);
                    config.set(Some(value));
                }
                Err(value) => error.set(value.message().into()),
            }
        });
    });
    let draft = move || -> Result<serde_json::Value, &'static str> {
        let Some(mut value) = config.get_untracked() else {
            return Err("请先载入语音配置。");
        };
        let rate = speed
            .get_untracked()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && (0.25..=4.0).contains(value))
            .ok_or("语速应在 0.25 到 4 之间。");
        value.speed = rate?;
        value.is_enabled = enabled.get_untracked();
        value.base_url = endpoint.get_untracked();
        value.model = model.get_untracked();
        value.voice = voice.get_untracked();
        if value.base_url.trim().is_empty()
            || value.model.trim().is_empty()
            || value.voice.trim().is_empty()
        {
            return Err("请填写服务地址、模型和声音 ID。");
        }
        Ok(value.draft(&key.get_untracked(), clear.get_untracked()))
    };
    let submit = move || {
        let body = match draft() {
            Ok(value) => value,
            Err(value) => {
                error.set(value.into());
                return;
            }
        };
        let Some(api) = api() else {
            return;
        };
        let generation = epoch.get_untracked();
        busy.set(true);
        error.set(String::new());
        notice.set(false);
        store.peek(|state| state.speech.clone()).stop();
        spawn(async move {
            let result = background(move || {
                api.write::<Config>(reqwest::Method::PUT, "api/v1/plugins/tts", &body)
            })
            .await;
            if epoch.get_untracked() != generation {
                return;
            }
            busy.set(false);
            match result {
                Ok(value) => {
                    config.set(Some(value));
                    wipe_key();
                    clear.set(false);
                    notice.set(true);
                }
                Err(value) => error.set(value.message().into()),
            }
        });
    };
    view()
        .gap_3()
        .child(ui::heading("语音朗读"))
        .child(ui::caption(
            "配置保存到所选资料库；试听使用未保存的配置，可能产生服务费用。",
        ))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(
                    ui::row(move || target.set(String::new()))
                        .selected(move || target.get().is_empty())
                        .child(text(move || crate::i18n::tr("本地资料库"))),
                )
                .child(each(
                    move || {
                        store.read(|state| {
                            state
                                .remotes
                                .iter()
                                .map(|remote| {
                                    (
                                        remote.profile.id.clone(),
                                        remote.profile.label(),
                                        remote.api.is_some(),
                                    )
                                })
                                .collect::<Vec<_>>()
                        })
                    },
                    |remote| remote.0.clone(),
                    move |remote, _| {
                        ui::row(move || {
                            target.set(remote.with_untracked(|remote| remote.0.clone()))
                        })
                        .selected(move || remote.with(|remote| remote.0 == target.get()))
                        .disabled(move || remote.with(|remote| !remote.2))
                        .child(text(move || remote.with(|remote| remote.1.clone())))
                    },
                )),
        )
        .child(when(
            move || target.with(|target| !target.is_empty()),
            || ui::caption("正在编辑远端配置；本机文章朗读使用本地配置。"),
        ))
        .child(when(move || busy.get(), || ui::caption("正在载入…")))
        .child(when(
            move || config.get().is_some(),
            move || {
                view()
                    .gap_3()
                    .child(
                        ui::row(move || enabled.update(|value| *value = !*value))
                            .selected(enabled)
                            .disabled(busy)
                            .child(text(move || crate::i18n::tr("启用语音朗读"))),
                    )
                    .child(labeled("语音服务地址", endpoint, busy))
                    .child(
                        view().gap_1().child(ui::caption("API 密钥（可选）")).child(
                            ui::field("API 密钥（可选）", key)
                                .masked(true)
                                .disabled(move || {
                                    busy.get()
                                        || clear.get()
                                        || config.with(|value| {
                                            value
                                                .as_ref()
                                                .is_none_or(|value| !value.keyring_available)
                                        })
                                }),
                        ),
                    )
                    .child(ui::caption(move || {
                        crate::i18n::tr(
                            if config
                                .with(|value| value.as_ref().is_some_and(|value| value.has_api_key))
                            {
                                "已保存密钥；留空保留。"
                            } else {
                                "未保存密钥；支持免密钥服务。"
                            },
                        )
                    }))
                    .child(when(
                        move || {
                            config.with(|value| {
                                value.as_ref().is_some_and(|value| !value.keyring_available)
                            })
                        },
                        || ui::caption("密钥存储不可用，可使用免密钥服务。"),
                    ))
                    .child(
                        ui::row(move || {
                            clear.update(|value| *value = !*value);
                            wipe_key();
                        })
                        .selected(clear)
                        .disabled(busy)
                        .child(text(move || crate::i18n::tr("清除已保存的密钥"))),
                    )
                    .child(labeled("语音模型", model, busy))
                    .child(ui::caption("声音"))
                    .child(
                        view()
                            .flex_row()
                            .flex_wrap()
                            .gap_2()
                            .children(VOICES.iter().map(move |&(id, label)| {
                                ui::row(move || {
                                    voice.set(id.into());
                                    custom.set(false);
                                })
                                .selected(move || !custom.get() && voice.get() == id)
                                .disabled(busy)
                                .child(text(move || crate::i18n::tr(label)).text_sm())
                            }))
                            .child(
                                ui::row(move || custom.set(true))
                                    .selected(custom)
                                    .disabled(busy)
                                    .child(text(move || crate::i18n::tr("自定义声音"))),
                            ),
                    )
                    .child(when(custom, move || labeled("声音 ID", voice, busy)))
                    .child(labeled("语速（0.25–4）", speed, busy))
                    .child(
                        view().gap_1().child(ui::caption("试听文本")).child(
                            ui::field("试听文本", preview_text)
                                .max_length(200)
                                .disabled(busy),
                        ),
                    )
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .child(ui::primary("保存语音配置", submit).disabled(busy))
                            .child(
                                ui::command("试听", move || match draft() {
                                    Ok(mut body) => {
                                        let text = preview_text.get_untracked();
                                        if text.trim().is_empty() || text.chars().count() > 200 {
                                            error.set("试听文本应为 1 到 200 个字符。".into());
                                            return;
                                        }
                                        body["text"] = text.into();
                                        error.set(String::new());
                                        if let Some(api) = api() {
                                            store
                                                .peek(|state| state.speech.clone())
                                                .preview(api, body);
                                        }
                                    }
                                    Err(value) => error.set(value.into()),
                                })
                                .disabled(move || {
                                    busy.get()
                                        || matches!(
                                            status.get(),
                                            Status::Generating | Status::Playing | Status::Paused
                                        )
                                }),
                            )
                            .child(
                                ui::command("停止试听", move || {
                                    store.peek(|state| state.speech.clone()).stop()
                                })
                                .disabled(move || {
                                    matches!(status.get(), Status::Idle | Status::Failed(_))
                                }),
                            ),
                    )
                    .child(feedback(status))
                    .child(when(notice, || ui::caption("语音配置已保存。")))
            },
        ))
        .child(when(
            move || !error.get().is_empty(),
            move || {
                text(move || crate::i18n::tr(&error.get()))
                    .text_sm()
                    .text_color(move || theme().destructive)
            },
        ))
        .child(when(
            move || config.get().is_none() && !busy.get(),
            move || {
                ui::command("重新载入语音配置", move || {
                    reload.update(|value| *value += 1);
                })
            },
        ))
}

#[cfg(test)]
mod tests;
