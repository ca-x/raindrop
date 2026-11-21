use nsh_base::text_view::Image;
use nsh_base::{
    TextViewLinkPressEvent, alert_dialog, dialog, h_resizable, menu, resizable_panel, text_view,
    theme,
};
use nsh_core::{element::ImageSrc, prelude::*};

use crate::{
    api::{Filter, Query, external_url},
    store::Store,
    ui,
};

const STAR_OUTLINE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" d="m10 2.75 2.2 4.46 4.92.72-3.56 3.47.84 4.9L10 14l-4.4 2.3.84-4.9-3.56-3.47 4.92-.72Z"/></svg>"#;
const STAR_FILLED: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" d="m10 2.75 2.2 4.46 4.92.72-3.56 3.47.84 4.9L10 14l-4.4 2.3.84-4.9-3.56-3.47 4.92-.72Z"/></svg>"#;

#[component]
pub(crate) fn Workspace() -> impl IntoElement {
    let store = use_context::<Store>().expect("workspace has a store");
    let adding = signal(false);
    let settings = signal(false);
    let show_sidebar = signal(false);
    let search = signal(String::new());
    let pending_search = signal(None::<String>);
    let current_query = computed(move || store.read(|state| state.query.clone()));
    effect(move || search.set(current_query.get().search));
    let search_ref = element_ref();
    let queue_ref = element_ref();
    let back_to_queue = move || {
        let index = store.peek(|state| {
            let id = state.selected.as_ref()?;
            state
                .entries
                .iter()
                .position(|row| &row.entry_id == id)
                .or_else(|| {
                    state
                        .adjacent_entry(1)
                        .or_else(|| state.adjacent_entry(-1))
                        .and_then(|next| {
                            state
                                .entries
                                .iter()
                                .position(|row| row.entry_id == next.entry_id)
                        })
                })
        });
        store.clear_selection();
        if window().width.0 < 760.0
            && let Some(index) = index
        {
            request_animation_frame(move |_| {
                queue_ref.scroll_to_index(index as u32, percentage(0.5), px(0.0), false)
            });
        }
    };
    let reader_active = move || !adding.get_untracked() && !settings.get_untracked();
    on_key("mod+f", move |_| {
        if reader_active() {
            if window().width.0 < 760.0 {
                show_sidebar.set(false);
                if store.peek(|state| state.selected.is_some()) {
                    store.clear_selection();
                }
                request_animation_frame(move |_| search_ref.focus());
            } else {
                search_ref.focus();
            }
        }
    });
    on_key("mod+r", move |_| {
        if reader_active() && !store.peek(|state| state.loading) {
            store.reload();
        }
    });
    on_key("escape", move |_| {
        if reader_active() {
            if show_sidebar.get_untracked() || pending_search.get_untracked().is_some() {
                show_sidebar.set(false);
                pending_search.set(None);
            } else {
                back_to_queue();
            }
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
                        .min_size(px(176.0))
                        .max_size(move || (window().width.0 >= 760.0).then_some(px(320.0)))
                        .visible(move || window().width.0 >= 1000.0 || show_sidebar.get())
                        .child(sidebar(
                            store,
                            adding,
                            settings,
                            show_sidebar,
                            pending_search,
                        )),
                )
                .child(
                    resizable_panel()
                        .size(px(336.0))
                        .min_size(px(256.0))
                        .max_size(move || (window().width.0 >= 760.0).then_some(px(480.0)))
                        .visible(move || {
                            window().width.0 >= 760.0
                                || (!show_sidebar.get()
                                    && store.read(|state| state.selected.is_none()))
                        })
                        .child(queue(
                            store,
                            adding,
                            search,
                            search_ref,
                            queue_ref,
                            show_sidebar,
                            pending_search,
                        )),
                )
                .child(
                    resizable_panel()
                        .visible(move || {
                            window().width.0 >= 760.0
                                || (!show_sidebar.get()
                                    && store.read(|state| state.selected.is_some()))
                        })
                        .child(article(store, back_to_queue)),
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
                .child(when(
                    move || window().width.0 >= 760.0,
                    || ui::caption("J / K 浏览 · M 已读 · S 收藏"),
                )),
        )
        .child(when(adding, move || add_subscription(store, adding)))
        .child(when(settings, crate::settings::settings))
}

fn sidebar(
    store: Store,
    adding: Signal<bool>,
    settings: Signal<bool>,
    showing: Signal<bool>,
    pending_search: Signal<Option<String>>,
) -> impl IntoElement {
    view()
        .flex_1()
        .min_h_0()
        .bg(move || theme().sidebar)
        .child(
            ui::toolbar()
                .child(when(
                    move || window().width.0 < 1000.0,
                    move || {
                        ui::command("返回", move || {
                            showing.set(false);
                            pending_search.set(None);
                        })
                    },
                ))
                .child(ui::heading("Raindrop").flex_1())
                .child(
                    ui::command("+", move || adding.set(true))
                        .accessibility_label(move || crate::i18n::tr("添加订阅")),
                ),
        )
        .child(view().px_2().py_3().gap_1().children(
            [Filter::All, Filter::Unread, Filter::Starred].map(move |filter| {
                ui::row(move || {
                    store.filter(filter, None);
                    pending_search.set(None);
                    showing.set(false);
                })
                .selected(move || {
                    store
                        .read(|state| state.query.filter == filter && state.query.feed_id.is_none())
                })
                .child(text(move || crate::i18n::tr(filter.label())).text_sm())
            }),
        ))
        .child(view().px_5().py_2().child(ui::caption(move || {
            crate::i18n::tr(if pending_search.with(Option::is_some) {
                "选择要搜索的订阅"
            } else {
                "订阅"
            })
        })))
        .child(
            virtual_list(
                move || store.read(|state| state.subscriptions.len()),
                move |at| {
                    let subscription = computed(move || {
                        store.read(|state| state.subscriptions.get(at.index()).cloned())
                    });
                    ui::row(move || {
                        if let Some(subscription) = subscription.get_untracked() {
                            store.set_query(Query {
                                feed_id: Some(subscription.feed_id),
                                search: pending_search.get_untracked().unwrap_or_default(),
                                filter: Filter::All,
                            });
                            pending_search.set(None);
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
                        .child(text(move || crate::i18n::tr("设置")).text_sm()),
                ),
        )
}

fn queue(
    store: Store,
    adding: Signal<bool>,
    search: Signal<String>,
    search_ref: ElementRef,
    list_ref: ElementRef,
    showing: Signal<bool>,
    pending_search: Signal<Option<String>>,
) -> impl IntoElement {
    view()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .child(
            ui::toolbar()
                .child(when(
                    move || window().width.0 < 1000.0,
                    move || ui::command("订阅", move || {
                        pending_search.set(None);
                        showing.update(|show| *show = !*show);
                    }),
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
                    .on_change(move |value: &str| {
                        search.set(value.to_owned());
                        pending_search.set(None);
                    })
                    .on_submit(move |value: &str| {
                        let term = value.trim();
                        if !term.is_empty() && store.peek(|state| state.query.feed_id.is_none()) {
                            pending_search.set(Some(term.to_owned()));
                            showing.set(true);
                        } else {
                            pending_search.set(None);
                            store.search(term.to_owned());
                        }
                    }),
            ),
        )
        .child(
            virtual_list(
                move || store.read(|state| state.entries.len()),
                move |at| {
                    let entry = computed(move || {
                        store.read(|state| state.entries.get(at.index()).cloned())
                    });
                    let saved = move || entry.with(|row| row.as_ref().is_some_and(|row| row.is_starred));
                    let label = move || entry.with(|row| row.as_ref().map(|row| {
                        let action = if row.is_starred { "取消收藏" } else { "收藏以便稍后阅读" };
                        format!("{} · {}", crate::i18n::tr(action), row.title())
                    }).unwrap_or_default());
                    let article = ui::row(move || {
                        entry.with_untracked(|row| {
                            if let Some(row) = row { store.select(row.entry_id.clone()); }
                        });
                    })
                    .flex_1().min_w_0()
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
                                            if entry.is_read { "" } else { "●" }
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
                                    .map(|entry| entry.title())
                                    .unwrap_or_default()
                            })
                        })
                        .text_sm()
                        .style_when(
                            move || entry.with(|entry| entry.as_ref().is_some_and(|row| !row.is_read)),
                            |s| s.font_weight(FontWeight::SEMIBOLD),
                        )
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
                    );
                    let star = ui::icon_command(
                        label,
                        move || ImageSrc::svg_markup(if saved() { STAR_FILLED } else { STAR_OUTLINE }),
                        move || {
                            entry.with_untracked(|row| {
                                if let Some(row) = row { store.toggle_entry_star(Some(&row.entry_id)); }
                            });
                        },
                    )
                    .min_w_10().min_h_10().selected(saved)
                    .disabled(move || store.read(|state| state.writing));
                    view().flex_row().items_center().mx_2().my_1().child(article).child(star)
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
                            } else if !state.query.search.is_empty() {
                                "没有匹配的文章"
                            } else if !state.subscriptions.is_empty() && state.query.filter == Filter::Unread {
                                "未读文章已读完"
                            } else if !state.subscriptions.is_empty() && state.query.filter == Filter::Starred {
                                "还没有收藏文章"
                            } else {
                                "这里还没有文章"
                            })
                        })
                    }))
                    .child(when(move || store.read(|state| !state.loading), move || {
                        when(move || store.read(|state| !state.query.search.is_empty()), move || {
                            ui::command("清除搜索", move || store.search(String::new()))
                        }).otherwise(move || {
                            when(move || store.read(|state| state.subscriptions.is_empty()), move || {
                                view().items_start().child(ui::primary("添加第一个订阅", move || adding.set(true)))
                            }).otherwise(move || {
                                when(move || store.read(|state| state.query.filter != Filter::All), move || {
                                    view().items_start().gap_2()
                                        .child(ui::caption(move || store.read(|state| crate::i18n::tr(
                                            if state.query.filter == Filter::Unread { "新文章到达后会出现在这里。" }
                                            else { "在列表中点亮星标，即可留到这里稍后阅读。" }
                                        ))))
                                        .child(ui::command("查看全部文章", move || store.filter(Filter::All, None)))
                                }).otherwise(|| ui::caption("添加订阅，或调整筛选条件。"))
                            })
                        })
                    })),
            ),
        )
}

fn article(store: Store, back_to_queue: impl Fn() + Copy + 'static) -> impl IntoElement {
    view()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .child(
            ui::toolbar()
                .flex_wrap()
                .child(when(
                    move || window().width.0 < 760.0,
                    move || ui::command("返回", back_to_queue),
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
                .child(crate::speech_ui::controls(store))
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
        .child(crate::speech_ui::article_feedback(store))
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
                                                .map(|entry| entry.title())
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
                                )
                                .child(article_continuation(store, back_to_queue)),
                        )
                    },
                )
            }),
        )
}

fn article_continuation(store: Store, back: impl Fn() + Copy + 'static) -> impl IntoElement {
    let next = computed(move || {
        store.read(|state| {
            state
                .adjacent_entry(1)
                .map(|entry| (entry.entry_id.clone(), entry.title()))
        })
    });
    view()
        .border_t_1()
        .border_color(move || theme().border)
        .pt_3()
        .mb_5()
        .child(
            when(
                move || next.with(Option::is_some),
                move || {
                    ui::row(move || {
                        if let Some((id, _)) = next.get_untracked() {
                            store.select(id);
                        }
                    })
                    .items_start()
                    .child(ui::caption("下一篇"))
                    .child(
                        text(move || {
                            next.with(|entry| {
                                entry
                                    .as_ref()
                                    .map(|(_, title)| title.clone())
                                    .unwrap_or_default()
                            })
                        })
                        .font_weight(FontWeight::SEMIBOLD)
                        .number_of_lines(2u32),
                    )
                },
            )
            .otherwise(move || {
                when(
                    move || store.read(|state| state.next_cursor.is_some()),
                    move || {
                        ui::command("加载更多文章", move || store.load_more())
                            .disabled(move || store.read(|state| state.loading))
                    },
                )
                .otherwise(move || ui::command("返回列表", back))
            }),
        )
}

fn add_subscription(store: Store, open: Signal<bool>) -> impl IntoElement {
    let address = signal(String::new());
    let submit = move || {
        let url = address.get_untracked();
        if !url.trim().is_empty() && !store.peek(|state| state.writing) {
            store.create_subscription(url);
            open.set(false);
        }
    };
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
                .child(ui::form_field(
                    "Feed 地址",
                    ui::field("Feed 地址", address)
                        .auto_focus(true)
                        .on_submit(move |_: &str| submit()),
                ))
                .child(ui::caption(
                    "默认只保存到本地。添加后可在设置的服务器连接页选择同步目标。",
                ))
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .justify_end()
                        .child(ui::command("取消", move || open.set(false)))
                        .child(ui::primary("添加", submit).disabled(move || {
                            address.with(|address| address.trim().is_empty())
                                || store.read(|state| state.writing)
                        })),
                ),
        )
}
