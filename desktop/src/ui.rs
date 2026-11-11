use nsh_base::{ButtonElement, MenuItemElement, button, input, menu_item, theme};
use nsh_core::element::{Text, View};
use nsh_core::prelude::*;

fn localized(label: impl IntoReactive<SharedString>) -> Reactive<SharedString> {
    match label.into_reactive() {
        Reactive::Static(label) => {
            Reactive::Bound(std::rc::Rc::new(move || crate::i18n::tr(&label)))
        }
        bound => bound,
    }
}

pub(crate) fn command(
    label: impl IntoReactive<SharedString> + 'static,
    action: impl Fn() + 'static,
) -> ButtonElement {
    button()
        .on_press(action)
        .min_h_8()
        .px_3()
        .py_1()
        .items_center()
        .justify_center()
        .rounded(move || theme().radius)
        .text_color(move || theme().foreground)
        .hover(|style| style.bg(move || theme().muted))
        .active(|style| style.bg(move || theme().accent))
        .focus(|style| style.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|style| style.opacity(0.45))
        .child(text(localized(label)).text_sm())
}

pub(crate) fn primary(
    label: impl IntoReactive<SharedString> + 'static,
    action: impl Fn() + 'static,
) -> ButtonElement {
    command(label, action)
        .bg(move || theme().primary)
        .text_color(move || theme().primary_foreground)
        .hover(|style| style.bg(move || theme().primary.opacity(0.9)))
        .active(|style| style.bg(move || theme().primary.opacity(0.8)))
}

pub(crate) fn row(action: impl Fn() + 'static) -> ButtonElement {
    button()
        .on_press(action)
        .min_h_8()
        .px_3()
        .py_2()
        .gap_1()
        .rounded(move || theme().radius)
        .text_color(move || theme().foreground)
        .hover(|style| style.bg(move || theme().muted))
        .active(|style| style.bg(move || theme().accent))
        .selected_style(|style| {
            style
                .bg(move || theme().accent)
                .text_color(move || theme().accent_foreground)
        })
        .focus(|style| style.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|style| style.opacity(0.45))
}

pub(crate) fn field(label: &'static str, value: Signal<String>) -> Input {
    input()
        .value(move || value.get())
        .on_change(move |next: &str| value.set(next.to_owned()))
        .label(move || crate::i18n::tr(label))
        .placeholder(move || crate::i18n::tr(label).to_string())
        .max_length(2048)
        .min_h_9()
        .px_3()
        .py_2()
        .text_sm()
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .border_1()
        .border_color(move || theme().input)
        .rounded(move || theme().radius)
        .focus(|style| {
            style
                .border_color(move || theme().ring)
                .outline(px(1.0))
                .outline_color(move || theme().ring)
        })
}

pub(crate) fn menu_command(label: &'static str, action: impl Fn() + 'static) -> MenuItemElement {
    menu_item()
        .on_select(action)
        .min_h_8()
        .px_3()
        .py_2()
        .rounded(move || theme().radius)
        .text_color(move || theme().popover_foreground)
        .hover(|style| style.bg(move || theme().accent))
        .focus(|style| style.bg(move || theme().accent))
        .active(|style| style.bg(move || theme().accent))
        .disabled_style(|style| style.opacity(0.45))
        .child(text(localized(label)).text_sm())
}

pub(crate) fn heading(label: impl IntoReactive<SharedString> + 'static) -> Text {
    text(localized(label))
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || theme().foreground)
}

pub(crate) fn caption(label: impl IntoReactive<SharedString> + 'static) -> Text {
    text(localized(label))
        .text_xs()
        .text_color(move || theme().muted_foreground)
}

pub(crate) fn toolbar() -> View {
    view()
        .flex_row()
        .items_center()
        .min_h_12()
        .px_3()
        .py_2()
        .gap_2()
        .border_b_1()
        .border_color(move || theme().border)
}

pub(crate) fn empty(title: &'static str, detail: &'static str) -> impl IntoElement {
    view()
        .flex_1()
        .items_center()
        .justify_center()
        .p_8()
        .gap_3()
        .child(heading(title))
        .child(
            text(move || crate::i18n::tr(detail))
                .text_sm()
                .text_center()
                .text_color(move || theme().muted_foreground),
        )
}
