//! The install-time consent screen (`docs/HUB.md` §7.3, phase H0 part 3).
//!
//! The decision logic is a pure function (`needs_consent`, in `plinth-hub`)
//! so it is tested without gpui. This module is the gpui view: it shows
//! the app name, the publisher, each capability that has no grant yet with
//! the capability text from the shared table and the app's reason, and per-capability Allow/Don't allow, then
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

/// One capability on the consent screen: its name, risk label, and the
/// decision the user is making for it. This screen only ever lists
/// Medium/High capabilities (`docs/HUB.md` §7.2): a None/Low one is
/// granted automatically by `Hub::add_package` and never reaches here
/// (`Hub::needs_consent`), so "allow" is simply the default while the
/// user decides.
struct Item {
    capability: String,
    risk_label: String,
    rationale: String,
    allowed: bool,
}

/// The consent screen's label for a risk level (`docs/HUB.md` §7.2).
fn risk_label(risk: plinth_link::capabilities::Risk) -> &'static str {
    match risk {
        plinth_link::capabilities::Risk::None => "No risk",
        plinth_link::capabilities::Risk::Low => "Low risk",
        plinth_link::capabilities::Risk::Medium => "Medium risk",
        plinth_link::capabilities::Risk::High => "High risk",
    }
}

/// What the user decided (`None` until Continue or Cancel).
enum Outcome {
    Cancelled,
    Continue(Vec<(String, bool)>),
}

/// The text for one capability (`docs/HUB.md` §7.2): the fixed description
/// from the shared capability table, the same for every app, then the
/// app's own reason from its manifest.
pub fn describe(capability: &str, app_rationale: &str) -> String {
    let fixed = match plinth_link::capabilities::info(capability) {
        Some(info) => {
            let mut d = info.description.to_owned();
            if let Some(first) = d.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            format!("{d}.")
        }
        None => format!("Uses {capability}."),
    };
    if app_rationale.trim().is_empty() { fixed } else { format!("{fixed} The app says: \u{201c}{}\u{201d}", app_rationale.trim()) }
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
        // Publishers are not signed yet (docs/HUB.md §6, phase H1), so the
        // manifest's publisher name is only a claim.
        let publisher_label = if self.publisher.trim().is_empty() {
            "Unverified publisher".to_owned()
        } else {
            format!("{} (unverified publisher)", self.publisher.trim())
        };

        let mut list = div().flex().flex_col().gap_3().w_full();
        for i in 0..self.items.len() {
            let allowed = self.items[i].allowed;
            let capability = self.items[i].capability.clone();
            let risk_label = self.items[i].risk_label.clone();
            let rationale = format!("{risk_label} \u{2014} {}", self.items[i].rationale);
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
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(t.text).child(capability))
                        .child(div().text_xs().text_color(t.text_muted).child(rationale)),
                )
                .child(
                    div()
                        .id(("consent-toggle", i))
                        .flex_none()
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
/// still need a decision, each with the app's rationale from its manifest;
/// see `plinth_hub::Hub::needs_consent`) and
/// returns the user's per-capability decisions, or `None` if they
/// cancelled. Blocks until the window closes: it runs its own gpui
/// application loop, so call it before opening the app's own window.
pub fn show(app_name: &str, publisher: &str, capabilities: &[(String, String)]) -> Option<Vec<(String, bool)>> {
    if capabilities.is_empty() {
        return Some(Vec::new());
    }
    let items: Vec<Item> = capabilities
        .iter()
        .map(|(c, why)| {
            let risk = plinth_link::capabilities::info(c).map(|i| i.risk).unwrap_or(plinth_link::capabilities::Risk::Medium);
            Item { capability: c.clone(), risk_label: risk_label(risk).to_owned(), rationale: describe(c, why), allowed: true }
        })
        .collect();
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
