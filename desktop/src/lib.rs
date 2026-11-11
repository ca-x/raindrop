use nsh_core::prelude::*;

mod api;
mod i18n;
mod local;
mod preferences;
mod reader;
mod settings;
mod store;
mod sync;
mod ui;

#[cfg(test)]
mod tests;

#[native_module("raindrop-desktop/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

/// The native Raindrop desktop application.
#[component]
pub fn App() -> impl IntoElement {
    set_tokens(
        nsh_base::tokens_of(Some(include_str!("../theme.json")))
            .expect("valid embedded desktop theme"),
    );
    let store = store::Store::new();
    provide_context(store);
    effect(move || store.start_local());
    view()
        .flex_1()
        .min_h_0()
        .bg(move || nsh_base::theme().background)
        .text_color(move || nsh_base::theme().foreground)
        .child(
            when(
                move || store.read(|state| state.api.is_some()),
                reader::workspace,
            )
            .otherwise(move || {
                view()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .p_8()
                    .gap_4()
                    .child(
                        text("Raindrop Desktop")
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(ui::caption(move || {
                        store.read(|state| {
                            if state.connecting {
                                crate::i18n::tr("正在打开本地资料库…")
                            } else {
                                crate::i18n::tr(&state.error)
                            }
                        })
                    }))
                    .child(when(
                        move || store.read(|state| !state.connecting),
                        move || ui::command("重试打开", move || store.start_local()),
                    ))
            }),
        )
}
