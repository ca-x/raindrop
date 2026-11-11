use nsh_base::text_view::Image;
use nsh_base::{
    TextViewLinkPressEvent, alert_dialog, dialog, h_resizable, menu, resizable_panel, text_view,
    theme,
};
use nsh_core::prelude::*;

use crate::{
    api::{Filter, external_url},
    store::Store,
    ui,
};

#[component]
pub(crate) fn Workspace() -> impl IntoElement {
    let store = use_context::<Store>().expect("workspace has a store");
    let adding = signal(false);
    let settings = signal(false);
    let show_sidebar = signal(false);
    let search = signal(String::new());
    let search_ref = element_ref();
    let queue_ref = element_ref();
    let reader_active = move || !adding.get_untracked() && !settings.get_untracked();
    on_key("mod+f", move |_| {
        if reader_active() {
            search_ref.focus();
        }
    });
    on_key("mod+r", move |_| {
        if reader_active() && !store.peek(|state| state.loading) {
            store.reload();
        }
    });
    on_key("escape", move |_| {
        if reader_active() {
            store.clear_selection();
        }
    });
    for (key, offset) in [("j", 1_isize), ("k", -1)] {
        on_key(key, move |_| {
            if !reader_active() {
                return;
            }
            store.move_selection(offset);
            if let Some(index) = store.peek(|state| {
                state
                    .selected
                    .as_ref()
                    .and_then(|id| state.entries.iter().position(|entry| &entry.entry_id == id))
            }) {
                queue_ref.scroll_to_index(index as u32, percentage(0.5), px(0.0), false);
            }
        });
    }
    on_key("m", move |_| {
        if reader_active() {
            store.toggle_read();
        }
    });
    on_key("s", move |_| {
        if reader_active() {
            store.toggle_star();
        }
    });
    provide_context(crate::settings::Visibility(settings));
    view()
        .flex_1()
        .min_h_0()
        .child(
            h_resizable()
                .flex_1()
                .min_h_0()
                .handle_class("w-px bg-border active:bg-ring")
                .child(
                    resizable_panel()
                        .size(px(224.0))
                        .size_range(px(176.0)..px(320.0))
                        .visible(move || window().width.0 >= 1000.0 || show_sidebar.get())
                        .child(sidebar(store, adding, settings, show_sidebar)),
                )
                .child(
                    resizable_panel()
                        .size(px(336.0))
                        .size_range(px(256.0)..px(480.0))
                        .visible(move || {
                            window().width.0 >= 760.0
                                || store.read(|state| state.selected.is_none())
                        })
                        .child(queue(store, search, search_ref, queue_ref, show_sidebar)),
                )
                .child(
                    resizable_panel()
                        .visible(move || {
                            window().width.0 >= 760.0
                                || store.read(|state| state.selected.is_some())
                        })
                        .child(article(store)),
                ),
        )
        .child(when(
            move || store.read(|state| !state.error.is_empty()),
            move || {
                view()
                    .flex_row()
                    .items_center()
                    .px_4()
                    .py_2()
                    .gap_3()
                    .bg(move || theme().muted)
                    .child(
                        text(move || store.read(|state| crate::i18n::tr(&state.error)))
                            .text_sm()
                            .text_color(move || theme().destructive)
                            .flex_1(),
                    )
                    .child(ui::command("重试", move || {
                        if let Some(id) = store.peek(|state| state.selected.clone()) {
                            store.select(id);
                        }
                        store.reload();
                    }))
            },
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .px_4()
                .py_2()
                .gap_2()
                .border_t_1()
                .border_color(move || theme().border)
                .child(ui::caption(move || {
                    store.read(|state| {
                        if state.loading {
                            crate::i18n::tr("正在载入…")
                        } else if !state.notice.is_empty() {
                            state.notice.localized()
                        } else {
                            crate::i18n::tr("{} 篇文章 · 本地存储")
                                .replace("{}", &state.entries.len().to_string())
                                .into()
                        }
                    })
                }))
                .child(view().flex_1())
                .child(ui::caption("J / K 浏览 · M 已读 · S 收藏")),
        )
        .child(when(adding, move || add_subscription(store, adding)))
        .child(when(settings, crate::settings::settings))
}

fn sidebar(
    store: Store,
    adding: Signal<bool>,
    settings: Signal<bool>,
    showing: Signal<bool>,
) -> impl IntoElement {
    view()
        .flex_1()
        .min_h_0()
        .bg(move || theme().sidebar)
        .child(
            ui::toolbar().child(ui::heading("Raindrop").flex_1()).child(
                ui::command("+", move || adding.set(true))
                    .accessibility_label(move || crate::i18n::tr("添加订阅")),
            ),
        )
        .child(view().px_2().py_3().gap_1().children(
            [Filter::All, Filter::Unread, Filter::Starred].map(move |filter| {
                ui::row(move || {
                    store.filter(filter, None);
                    showing.set(false);
                })
                .selected(move || {
                    store
                        .read(|state| state.query.filter == filter && state.query.feed_id.is_none())
                })
                .child(text(move || crate::i18n::tr(filter.label())).text_sm())
            }),
        ))
        .child(view().px_5().py_2().child(ui::caption("订阅")))
        .child(
            virtual_list(
                move || store.read(|state| state.subscriptions.len()),
                move |at| {
                    let subscription = computed(move || {
                        store.read(|state| state.subscriptions.get(at.index()).cloned())
                    });
                    ui::row(move || {
                        if let Some(subscription) = subscription.get_untracked() {
                            store.filter(Filter::All, Some(subscription.feed_id));
                            showing.set(false);
                        }
                    })
                    .mx_2()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .selected(move || {
                        subscription.with(|subscription| {
                            subscription.as_ref().is_some_and(|subscription| {
                                store.read(|state| {
                                    state.query.feed_id.as_ref() == Some(&subscription.feed_id)
                                })
                            })
                        })
                    })
                    .child(
                        text(move || {
                            subscription.with(|subscription| {
                                subscription
                                    .as_ref()
                                    .map(|subscription| subscription.title.clone())
                                    .unwrap_or_default()
                            })
                        })
                        .text_sm()
                        .truncate()
                        .flex_1(),
                    )
                    .child(ui::caption(move || {
                        subscription.with(|subscription| {
                            subscription
                                .as_ref()
                                .filter(|subscription| subscription.unread_count > 0)
                                .map(|subscription| subscription.unread_count.to_string())
                                .unwrap_or_default()
                        })
                    }))
                },
            )
            .keys(move || {
                store.read(|state| {
                    state
                        .subscriptions
                        .iter()
                        .map(|subscription| subscription.subscription_id.clone())
                        .collect::<Vec<_>>()
                })
            })
            .estimated_item_size(px(36.0))
            .flex_1()
            .min_h_0()
            .empty(ui::caption("尚无订阅").px_5().py_3()),
        )
        .child(
            view()
                .p_2()
                .border_t_1()
                .border_color(move || theme().border)
                .child(
                    ui::row(move || settings.set(true))
                        .child(text(move || crate::i18n::tr("服务器与同步")).text_sm()),
                ),
        )
}

fn queue(
    store: Store,
    search: Signal<String>,
    search_ref: ElementRef,
    list_ref: ElementRef,
    showing: Signal<bool>,
) -> impl IntoElement {
    view()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .child(
            ui::toolbar()
                .child(when(
                    move || window().width.0 < 1000.0,
                    move || ui::command("订阅", move || showing.update(|show| *show = !*show)),
                ))
                .child(
                    ui::heading(move || {
                        store.read(|state| {
                            state
                                .query
                                .feed_id
                                .as_ref()
                                .and_then(|feed| {
                                    state
                                        .subscriptions
                                        .iter()
                                        .find(|subscription| &subscription.feed_id == feed)
                                })
                                .map(|subscription| subscription.title.clone())
                                .unwrap_or_else(|| crate::i18n::tr(state.query.filter.label()).to_string())
                        })
                    })
                    .truncate()
                    .flex_1(),
                )
                .child(
                    menu()
                        .trigger(ui::command("…", || {}).accessibility_label(move || crate::i18n::tr("文章列表操作")))
                        .popup_class(
                            "min-w-48 p-1 rounded-md border border-border bg-popover shadow-lg",
                        )
                        .child(
                            ui::menu_command("重新载入文章", move || store.reload())
                                .disabled(move || store.read(|state| state.loading)),
                        )
                        .child(
                            ui::menu_command("抓取当前订阅", move || store.refresh_current())
                                .disabled(move || {
                                    store.read(|state| {
                                        state.query.feed_id.is_none() || state.writing
                                    })
                                }),
                        )
                        .child(
                            ui::menu_command("取消本地订阅…", move || {
                                store.spawn(async move {
                                    let result = alert_dialog().title(crate::i18n::tr("取消本地订阅？").to_string())
                                        .description(crate::i18n::tr("只删除此设备上的订阅，不删除远端；已导入的来源不会在下次拉取时自动添加回来。").to_string())
                                        .cancel("cancel", crate::i18n::tr("保留").to_string()).destructive("remove", crate::i18n::tr("取消订阅").to_string()).show().await;
                                    if result.is_ok_and(|result| result.action.as_deref() == Some("remove")) { store.remove_current(); }
                                });
                            }).disabled(move || store.read(|state| state.query.feed_id.is_none() || state.writing || state.sync_busy))
                        ),
                ),
        )
        .child(
            view().px_3().py_2().child(
                ui::field("搜索文章", search)
                    .element_ref(&search_ref)
                    .on_submit(move |_: &str| store.search(search.get_untracked())),
            ),
        )
        .child(
            virtual_list(
                move || store.read(|state| state.entries.len()),
                move |at| {
                    let entry = computed(move || {
                        store.read(|state| state.entries.get(at.index()).cloned())
                    });
                    ui::row(move || {
                        if let Some(entry) = entry.get_untracked() {
                            store.select(entry.entry_id);
                        }
                    })
                    .mx_2()
                    .my_1()
                    .px_3()
                    .py_3()
                    .gap_2()
                    .selected(move || {
                        entry.with(|entry| {
                            entry.as_ref().is_some_and(|entry| {
                                store.read(|state| state.selected.as_ref() == Some(&entry.entry_id))
                            })
                        })
                    })
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                ui::caption(move || {
                                    entry.with(|entry| {
                                        entry
                                            .as_ref()
                                            .map(|entry| entry.feed_title.clone())
                                            .unwrap_or_default()
                                    })
                                })
                                .truncate()
                                .flex_1(),
                            )
                            .child(ui::caption(move || {
                                entry.with(|entry| {
                                    entry
                                        .as_ref()
                                        .map(|entry| {
                                            if entry.is_starred {
                                                "★"
                                            } else if !entry.is_read {
                                                "●"
                                            } else {
                                                ""
                                            }
                                        })
                                        .unwrap_or_default()
                                })
                            })),
                    )
                    .child(
                        text(move || {
                            entry.with(|entry| {
                                entry
                                    .as_ref()
                                    .map(|entry| entry.title().to_owned())
                                    .unwrap_or_default()
                            })
                        })
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .number_of_lines(2u32),
                    )
                    .child(
                        ui::caption(move || {
                            entry.with(|entry| {
                                entry
                                    .as_ref()
                                    .and_then(|entry| entry.summary.clone())
                                    .unwrap_or_default()
                            })
                        })
                        .number_of_lines(2u32),
                    )
                },
            )
            .keys(move || {
                store.read(|state| {
                    state
                        .entries
                        .iter()
                        .map(|entry| entry.entry_id.clone())
                        .collect::<Vec<_>>()
                })
            })
            .estimated_item_size(px(120.0))
            .element_ref(&list_ref)
            .flex_1()
            .min_h_0()
            .on_load_more(move || store.load_more())
            .empty(
                view()
                    .p_6()
                    .gap_2()
                    .child(ui::heading(move || {
                        store.read(|state| {
                            crate::i18n::tr(if state.loading {
                                "正在载入…"
                            } else {
                                "这里还没有文章"
                            })
                        })
                    }))
                    .child(ui::caption("添加订阅，或调整筛选条件。")),
            ),
        )
}

fn article(store: Store) -> impl IntoElement {
    view()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .child(
            ui::toolbar()
                .child(when(
                    move || window().width.0 < 760.0,
                    move || ui::command("返回", move || store.clear_selection()),
                ))
                .child(
                    ui::command(
                        move || {
                            store.read(|state| {
                                crate::i18n::tr(
                                    if state.detail.as_ref().is_some_and(|entry| entry.is_read) {
                                        "标为未读"
                                    } else {
                                        "标为已读"
                                    },
                                )
                            })
                        },
                        move || store.toggle_read(),
                    )
                    .disabled(move || store.read(|state| state.detail.is_none() || state.writing)),
                )
                .child(
                    ui::command(
                        move || {
                            store.read(|state| {
                                crate::i18n::tr(
                                    if state.detail.as_ref().is_some_and(|entry| entry.is_starred) {
                                        "取消收藏"
                                    } else {
                                        "收藏文章"
                                    },
                                )
                            })
                        },
                        move || store.toggle_star(),
                    )
                    .disabled(move || store.read(|state| state.detail.is_none() || state.writing)),
                )
                .child(view().flex_1())
                .child(
                    ui::command("打开原文", move || {
                        if let Some(url) = store.peek(|state| {
                            state.detail.as_ref().and_then(|entry| {
                                entry.canonical_url.as_deref().and_then(external_url)
                            })
                        }) {
                            spawn(async move {
                                let _ = open_url(url).await;
                            });
                        }
                    })
                    .disabled(move || {
                        store.read(|state| {
                            state
                                .detail
                                .as_ref()
                                .and_then(|entry| {
                                    entry.canonical_url.as_deref().and_then(external_url)
                                })
                                .is_none()
                        })
                    }),
                ),
        )
        .child(
            when(
                move || store.read(|state| state.detail.is_none()),
                move || {
                    when(
                        move || store.read(|state| state.loading_detail),
                        || ui::empty("正在载入文章…", "请稍候。"),
                    )
                    .otherwise(|| {
                        ui::empty("留一点时间，读点好内容", "从列表中选择一篇文章开始阅读。")
                    })
                },
            )
            .otherwise(move || {
                let selected = move || {
                    store.read(|state| {
                        state
                            .detail
                            .as_ref()
                            .map(|entry| entry.entry_id.clone())
                            .unwrap_or_default()
                    })
                };
                each(
                    move || vec![selected()],
                    |id: &String| id.clone(),
                    move |_id, _index| {
                        scroll().flex_1().min_h_0().child(
                            view()
                                .px_8()
                                .py_8()
                                .gap_5()
                                .min_w_0()
                                .child(ui::caption(move || {
                                    store.read(|state| {
                                        state
                                            .detail
                                            .as_ref()
                                            .map(|entry| {
                                                format!(
                                                    "{}{}",
                                                    entry.feed_title,
                                                    entry
                                                        .author
                                                        .as_ref()
                                                        .map(|author| format!(" · {author}"))
                                                        .unwrap_or_default()
                                                )
                                            })
                                            .unwrap_or_default()
                                    })
                                }))
                                .child(
                                    text(move || {
                                        store.read(|state| {
                                            state
                                                .detail
                                                .as_ref()
                                                .map(|entry| entry.title().to_owned())
                                                .unwrap_or_default()
                                        })
                                    })
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD),
                                )
                                .child(
                                    text_view()
                                        .html(move || {
                                            store.read(|state| {
                                                state
                                                    .detail
                                                    .as_ref()
                                                    .map(|entry| entry.content_html.clone().into())
                                            })
                                        })
                                        .selectable(true)
                                        .text_base()
                                        .gap_4()
                                        .text_color(move || theme().foreground)
                                        .render_image(|image: &Image| {
                                            ui::caption(if image.alt.is_empty() {
                                                crate::i18n::tr("[图片：请打开原文查看]")
                                                    .to_string()
                                            } else {
                                                crate::i18n::tr("[图片：{}]")
                                                    .replace("{}", &image.alt)
                                            })
                                        })
                                        .on_link_press(|event: &TextViewLinkPressEvent| {
                                            if let Some(url) = external_url(&event.url) {
                                                spawn(async move {
                                                    let _ = open_url(url).await;
                                                });
                                            }
                                        }),
                                ),
                        )
                    },
                )
            }),
        )
}

fn add_subscription(store: Store, open: Signal<bool>) -> impl IntoElement {
    let address = signal(String::new());
    dialog()
        .on_cancel(move || open.set(false))
        .animation(ModalAnimation::None)
        .flex_1()
        .items_center()
        .justify_center()
        .p_6()
        .bg(move || theme().muted)
        .child(
            view()
                .w(px(520.0))
                .max_w(relative(1.0))
                .bg(move || theme().background)
                .border_1()
                .border_color(move || theme().border)
                .rounded(move || theme().radius)
                .p_6()
                .gap_4()
                .child(
                    text(move || crate::i18n::tr("添加订阅"))
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(ui::field("Feed 地址", address).auto_focus(true))
                .child(ui::caption(
                    "默认只保存到本地。添加后可在服务器与同步中选择同步目标。",
                ))
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .justify_end()
                        .child(ui::command("取消", move || open.set(false)))
                        .child(
                            ui::primary("添加", move || {
                                store.create_subscription(address.get_untracked());
                                open.set(false);
                            })
                            .disabled(move || {
                                address.with(|address| address.trim().is_empty())
                                    || store.read(|state| state.writing)
                            }),
                        ),
                ),
        )
}
