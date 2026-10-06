//! The install-time consent screen (`docs/HUB.md` §7.3, phase H0 part 3).
//!
//! The decision logic is a pure function (`needs_consent`, in `plinth-hub`)
//! so it is tested without gpui. This module is the gpui view: it shows
//! the app name, the publisher, each capability that has no grant yet with
//! a placeholder rationale, and per-capability Allow/Don't allow, then
//! Continue/Cancel.
//!
//! The capability wording table (`docs/HUB.md` §7.2) is being built in
//! parallel in `plinth-link`; until it lands, `rationale_for` below is a
//! small placeholder that is easy to swap for the shared table.

use gpui::{
    App, AppContext, Bounds, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    StatefulInteractiveElement, Window, WindowBounds, WindowOptions, actions, div, px, size,
};
use plinth_ui::Tokens;
use std::cell::Cell;
use std::rc::Rc;

/// One capability on the consent screen: its name and the decision the
/// user is making for it (defaults to "allow", matching §7.3's "low-risk
/// capabilities are granted with the install").
struct Item {
    capability: String,
    rationale: String,
    allowed: bool,
}

/// What the user decided (`None` until Continue or Cancel).
enum Outcome {
    Cancelled,
    Continue(Vec<(String, bool)>),
}

/// A short, fixed-format rationale for a capability (`docs/HUB.md` §7.2).
/// Placeholder until the shared capability table in `plinth-link` lands;
/// swapping it for that table only needs this function to change.
pub fn rationale_for(capability: &str) -> String {
    match capability {
        "store.kv" => "Save data on this device.".to_owned(),
        "clipboard.read" => "Read what you last copied.".to_owned(),
        "clipboard.write" => "Copy text for you.".to_owned(),
        "notify" => "Show notifications.".to_owned(),
        _ => format!("Uses {capability}."),
    }
}

struct ConsentView {
    app_name: String,
    publisher: String,
    items: Vec<Item>,
    result: Rc<Cell<Option<Outcome>>>,
}

impl ConsentView {
    fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(item) = self.items.get_mut(index) {
            item.allowed = !item.allowed;
            cx.notify();
        }
    }

    fn finish(&mut self, outcome: Outcome, window: &mut Window, cx: &mut Context<Self>) {
        self.result.set(Some(outcome));
        window.remove_window();
        let _ = cx;
    }
}

impl Render for ConsentView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Tokens::new(window.appearance(), "teal");
        let publisher_label = if self.publisher.trim().is_empty() { "Unverified publisher".to_owned() } else { self.publisher.clone() };

        let mut list = div().flex().flex_col().gap_3().w_full();
        for i in 0..self.items.len() {
            let allowed = self.items[i].allowed;
            let capability = self.items[i].capability.clone();
            let rationale = self.items[i].rationale.clone();
            let row = div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(t.border)
                .bg(t.surface)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(t.text).child(capability))
                        .child(div().text_xs().text_color(t.text_muted).child(rationale)),
                )
                .child(
                    div()
                        .id(("consent-toggle", i))
                        .cursor_pointer()
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .border_1()
                        .border_color(if allowed { t.accent } else { t.border })
                        .bg(if allowed { t.accent } else { t.background })
                        .text_xs()
                        .text_color(if allowed { t.on_accent } else { t.text_muted })
                        .child(if allowed { "Allow" } else { "Don't allow" })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(i, cx))),
                );
            list = list.child(row);
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.background)
            .p_6()
            .gap_4()
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(self.app_name.clone()))
            .child(div().text_sm().text_color(t.text_muted).child(publisher_label))
            .child(div().text_sm().text_color(t.text).child("This app asks for:"))
            .child(list)
            .child(
                div()
                    .flex()
                    .gap_3()
                    .mt_2()
                    .child(
                        div()
                            .id("consent-cancel")
                            .cursor_pointer()
                            .px_4()
                            .py_2()
                            .rounded_md()
                            .border_1()
                            .border_color(t.border)
                            .text_sm()
                            .text_color(t.text)
                            .child("Cancel")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.finish(Outcome::Cancelled, window, cx))),
                    )
                    .child(
                        div()
                            .id("consent-continue")
                            .cursor_pointer()
                            .px_4()
                            .py_2()
                            .rounded_md()
                            .bg(t.accent)
                            .text_sm()
                            .text_color(t.on_accent)
                            .child("Continue")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                let decisions = this.items.iter().map(|i| (i.capability.clone(), i.allowed)).collect();
                                this.finish(Outcome::Continue(decisions), window, cx)
                            })),
                    ),
            )
    }
}

actions!(plinth_consent, [ConsentQuit]);

/// Shows the consent screen for `capabilities` (the capabilities that
/// still need a decision; see `plinth_hub::Hub::needs_consent`) and
/// returns the user's per-capability decisions, or `None` if they
/// cancelled. Blocks until the window closes: it runs its own gpui
/// application loop, so call it before opening the app's own window.
pub fn show(app_name: &str, publisher: &str, capabilities: &[String]) -> Option<Vec<(String, bool)>> {
    if capabilities.is_empty() {
        return Some(Vec::new());
    }
    let items: Vec<Item> =
        capabilities.iter().map(|c| Item { capability: c.clone(), rationale: rationale_for(c), allowed: true }).collect();
    let result = Rc::new(Cell::new(None));
    let app_name = app_name.to_owned();
    let publisher = publisher.to_owned();
    let out = result.clone();

    gpui_platform::application().run(move |cx: &mut App| {
        plinth_ui::init(cx);
        cx.on_action(|_: &ConsentQuit, cx| cx.quit());
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(480.), px(420.)), cx);
        let options = WindowOptions::new()
            .window_bounds(Some(WindowBounds::Windowed(bounds)))
            .titlebar(Some(gpui::TitlebarOptions { title: Some("Allow this app?".into()), ..Default::default() }));
        cx.open_window(options, move |_, cx| {
            cx.new(|_| ConsentView { app_name: app_name.clone(), publisher: publisher.clone(), items, result: out.clone() })
        })
        .expect("open the consent window");
        cx.activate(true);
    });

    match result.take() {
        Some(Outcome::Continue(decisions)) => Some(decisions),
        Some(Outcome::Cancelled) | None => None,
    }
}
