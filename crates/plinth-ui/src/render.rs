//! The gpui-ce view that renders the semantic tree (SPEC.md §6 and §9.3).
//!
//! The runtime owns all layout, spacing and color. Apps only supply the tree.

use crate::theme::{Tokens, WidthClass, icon_glyph};
use crate::tree::{Node, Tree};
use gpui::{
    AnyElement, ClickEvent, Context, ElementId, Entity, FontWeight, IntoElement, Render,
    SharedString, Subscription, Window, div, prelude::*, px,
};
use gpui_elements::editable_text::actions::Enter;
use gpui_elements::editable_text::{EditableTextState, StringStorage, TextChanged, text_input};
use plinth_protocol::{
    ControlKind, Event, NodeId, Value, Writer, axis, button_role, button_size, event, prop, text_align, text_style, tone,
};
use std::collections::HashMap;
use std::time::Instant;

/// The host side of the guest connection. `dispatch` sends one event buffer
/// and returns the op buffers that the guest committed while it ran.
pub trait GuestPort {
    fn dispatch(&mut self, events: &[u8]) -> anyhow::Result<Vec<Vec<u8>>>;
}

/// Host-side state of one `TextField`.
struct Field {
    state: Entity<EditableTextState>,
    /// The last value that the host and the guest agree on.
    last_synced: String,
    _subscription: Subscription,
}

pub struct PlinthRoot {
    tree: Tree,
    guest: Box<dyn GuestPort>,
    accent: SharedString,
    fields: HashMap<NodeId, Field>,
    /// Set when the guest traps. The tree then stays read-only.
    stopped: Option<String>,
    class: WidthClass,
}

impl PlinthRoot {
    /// Makes the view. `initial_commits` are the op buffers from `init`.
    pub fn new(
        guest: Box<dyn GuestPort>,
        initial_commits: Vec<Vec<u8>>,
        accent: impl Into<SharedString>,
        _cx: &mut Context<Self>,
    ) -> Self {
        let mut root = Self {
            tree: Tree::new(),
            guest,
            accent: accent.into(),
            fields: HashMap::new(),
            stopped: None,
            class: WidthClass::Wide,
        };
        root.apply_commits(initial_commits);
        root
    }

    /// Makes the view for a guest that trapped in `init`.
    pub fn stopped(guest: Box<dyn GuestPort>, error: String, accent: impl Into<SharedString>) -> Self {
        Self {
            tree: Tree::new(),
            guest,
            accent: accent.into(),
            fields: HashMap::new(),
            stopped: Some(error),
            class: WidthClass::Wide,
        }
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// Shows the screen with the given index (`plinth shoot` uses it).
    pub fn select_screen(&mut self, screen: u32, cx: &mut Context<Self>) {
        self.tree.current_screen = screen;
        cx.notify();
    }

    /// Replaces the guest with a new build (hot reload). The selected screen
    /// stays; the guest state starts again.
    pub fn reload(&mut self, guest: Box<dyn GuestPort>, init: Result<Vec<Vec<u8>>, String>, cx: &mut Context<Self>) {
        let screen = self.tree.current_screen;
        self.guest = guest;
        self.tree = Tree::new();
        self.fields.clear();
        self.stopped = None;
        match init {
            Ok(commits) => self.apply_commits(commits),
            Err(e) => self.stopped = Some(e),
        }
        if self.tree.screens().any(|(s, _)| s == screen) {
            self.tree.current_screen = screen;
        }
        cx.notify();
    }

    fn apply_commits(&mut self, commits: Vec<Vec<u8>>) {
        for commit in commits {
            match self.tree.apply(&commit) {
                Ok(errors) => {
                    for e in errors {
                        log::warn!("guest op {} skipped: {}", e.index, e.message);
                    }
                }
                Err(e) => log::error!("guest sent a malformed commit ({e}); commit ignored"),
            }
        }
        for id in self.tree.take_removed() {
            self.fields.remove(&id);
        }
    }

    /// Sends one `ui` event to the guest and applies what it commits.
    fn fire(&mut self, handler: u32, ev: u16, value: Value, cx: &mut Context<Self>) {
        if self.stopped.is_some() {
            return;
        }
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: ev, value });
        let start = Instant::now();
        match self.guest.dispatch(w.as_bytes()) {
            Ok(commits) => {
                self.apply_commits(commits);
                log::debug!("event round trip: {:?}", start.elapsed());
            }
            Err(e) => {
                log::error!("the app stopped: {e:#}");
                self.stopped = Some(format!("{e:#}"));
            }
        }
        self.sync_fields(cx);
        cx.notify();
    }

    /// Pushes guest-side `value` changes into the text field states.
    fn sync_fields(&mut self, cx: &mut Context<Self>) {
        for (id, field) in self.fields.iter_mut() {
            let guest_value = self.tree.get(*id).and_then(|n| n.str_prop(prop::VALUE)).unwrap_or("");
            if guest_value != field.last_synced {
                field.last_synced = guest_value.to_owned();
                let value = field.last_synced.clone();
                field.state.update(cx, |s, cx| s.emplace(&value, cx));
            }
        }
    }

    fn on_text_changed(&mut self, id: NodeId, state: Entity<EditableTextState>, cx: &mut Context<Self>) {
        let text = state.read(cx).as_str().to_owned();
        let Some(field) = self.fields.get_mut(&id) else { return };
        if text == field.last_synced {
            return; // The host set this value, so the guest knows it already.
        }
        field.last_synced = text.clone();
        self.tree.set_local_prop(id, prop::VALUE, Value::Str(text.clone()));
        if let Some(handler) = self.tree.get(id).and_then(|n| n.handler(event::CHANGE)) {
            self.fire(handler, event::CHANGE, Value::Str(text), cx);
        }
    }

    /// Makes the text field states for visible `TextField` nodes. This must
    /// happen before the element pass, because it needs `&mut self`.
    fn ensure_fields(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.tree.current_root() else { return };
        let mut stack = vec![root.id];
        let mut missing = Vec::new();
        while let Some(id) = stack.pop() {
            let Some(node) = self.tree.get(id) else { continue };
            if node.kind == Some(ControlKind::TextField) && !self.fields.contains_key(&id) {
                missing.push((id, node.str_prop(prop::VALUE).unwrap_or("").to_owned()));
            }
            stack.extend(node.children.iter().copied());
        }
        for (id, value) in missing {
            let initial = value.clone();
            let state = cx.new(|cx| EditableTextState::new(StringStorage::from(initial), cx));
            let subscription =
                cx.subscribe(&state, move |this, state, _: &TextChanged, cx| this.on_text_changed(id, state, cx));
            self.fields.insert(id, Field { state, last_synced: value, _subscription: subscription });
        }
    }
}

fn eid(prefix: &'static str, id: impl Into<u64>) -> ElementId {
    ElementId::NamedInteger(prefix.into(), id.into())
}

impl Render for PlinthRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.class = WidthClass::from_width(window.viewport_size().width);
        self.ensure_fields(cx);
        let t = Tokens::new(window.appearance(), &self.accent);

        let content = div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .child(self.render_stopped_banner(&t))
            .child(match self.tree.current_root() {
                Some(root) => self.render_screen(root, &t, cx),
                None => div().flex_1().into_any_element(),
            });

        let screens: Vec<(u32, NodeId)> = self.tree.screens().collect();
        let shell = div().size_full().flex().bg(t.background).text_color(t.text);
        if screens.len() < 2 {
            return shell.child(content);
        }
        match self.class {
            WidthClass::Compact => {
                shell.flex_col().child(content).child(self.render_nav(&screens, &t, cx))
            }
            WidthClass::Regular | WidthClass::Wide => {
                shell.flex_row().child(self.render_nav(&screens, &t, cx)).child(content)
            }
        }
    }
}

impl PlinthRoot {
    fn render_stopped_banner(&self, t: &Tokens) -> impl IntoElement {
        div().when_some(self.stopped.clone(), |d, error| {
            d.w_full()
                .px_4()
                .py_3()
                .bg(t.danger)
                .text_color(t.on_accent)
                .flex()
                .flex_col()
                .gap_1()
                .child(div().font_weight(FontWeight::SEMIBOLD).child("This app stopped"))
                .child(div().text_xs().child(error))
        })
    }

    /// The navigation shell (SPEC.md §6.2): a tab bar on compact, a rail on
    /// regular and a sidebar on wide.
    fn render_nav(&self, screens: &[(u32, NodeId)], t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let current = self.tree.current_root().map(|n| n.id);
        let items = screens.iter().map(|&(screen, id)| {
            let node = self.tree.get(id);
            let title: SharedString = node.and_then(|n| n.str_prop(prop::TITLE)).unwrap_or("Untitled").to_owned().into();
            let glyph = icon_glyph(node.and_then(|n| n.str_prop(prop::ICON)).unwrap_or(""));
            let selected = current == Some(id);
            let fg = if selected { t.accent } else { t.text_muted };
            let item = div()
                .id(eid("nav", screen))
                .cursor_pointer()
                .text_color(fg)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.tree.current_screen = screen;
                    cx.notify();
                }));
            match self.class {
                WidthClass::Compact | WidthClass::Regular => item
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_0p5()
                    .py_2()
                    .when(self.class == WidthClass::Compact, |d| d.flex_1())
                    .when(self.class == WidthClass::Regular, |d| d.w_full())
                    .child(div().text_lg().child(glyph))
                    .child(div().text_xs().child(title)),
                WidthClass::Wide => item
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .when(selected, |d| d.bg(t.selected))
                    .hover(|s| s.bg(t.hover))
                    .child(div().w(px(20.)).child(glyph))
                    .child(div().text_sm().child(title)),
            }
        });

        let bar = div().flex().bg(t.surface).border_color(t.border);
        match self.class {
            WidthClass::Compact => bar.flex_row().h(px(60.)).border_t_1().children(items).into_any_element(),
            WidthClass::Regular => bar.flex_col().w(px(84.)).pt_4().gap_2().border_r_1().children(items).into_any_element(),
            WidthClass::Wide => bar.flex_col().w(px(232.)).p_3().gap_1().border_r_1().children(items).into_any_element(),
        }
    }

    fn render_screen(&self, screen: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let title = screen.str_prop(prop::TITLE).unwrap_or("").to_owned();
        let children: Vec<AnyElement> = screen.children.iter().map(|&c| self.render_node(c, t, cx)).collect();
        let pad = if self.class == WidthClass::Compact { px(16.) } else { px(32.) };
        div()
            .id(eid("screen", screen.id))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(720.))
                    .px(pad)
                    .py_6()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(div().text_2xl().font_weight(FontWeight::BOLD).child(title))
                    .children(children),
            )
            .into_any_element()
    }

    fn render_children(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> Vec<AnyElement> {
        node.children.iter().map(|&c| self.render_node(c, t, cx)).collect()
    }

    fn render_node(&self, id: NodeId, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let Some(node) = self.tree.get(id) else { return div().into_any_element() };
        let Some(kind) = node.kind else {
            return div()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(t.border)
                .text_xs()
                .text_color(t.text_muted)
                .child(format!("Unsupported control (kind {})", node.raw_kind))
                .into_any_element();
        };
        match kind {
            ControlKind::Screen => self.render_screen(node, t, cx),
            ControlKind::Section => self.render_section(node, t, cx),
            ControlKind::Text => self.render_text(node, t),
            ControlKind::Heading => self.render_heading(node),
            ControlKind::Button => self.render_button(node, t, cx),
            ControlKind::TextField => self.render_text_field(node, t, cx),
            ControlKind::Toggle => self.render_toggle(node, t, cx),
            ControlKind::List => self.render_list(node, t, cx),
            ControlKind::Row => self.render_row(node, t, cx),
            ControlKind::Empty => self.render_empty(node, t),
            ControlKind::Group => self.render_group(node, t, cx),
        }
    }

    fn render_section(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let title = node.str_prop(prop::TITLE).map(str::to_uppercase);
        let footer = node.str_prop(prop::FOOTER).map(str::to_owned);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .when_some(title, |d, title| d.child(div().px_1().text_xs().text_color(t.text_muted).child(title)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .rounded_xl()
                    .bg(t.surface)
                    .border_1()
                    .border_color(t.border)
                    .children(self.render_children(node, t, cx)),
            )
            .when_some(footer, |d, footer| d.child(div().px_1().text_xs().text_color(t.text_muted).child(footer)))
            .into_any_element()
    }

    fn render_text(&self, node: &Node, t: &Tokens) -> AnyElement {
        let color = match node.enum_prop(prop::TONE) {
            tone::MUTED => t.text_muted,
            tone::DANGER => t.danger,
            tone::SUCCESS => t.success,
            _ => t.text,
        };
        let text = node.text.clone().unwrap_or_default();
        let d = aligned(div().text_color(color), node);
        match node.enum_prop(prop::STYLE) {
            text_style::CAPTION => d.text_xs(),
            text_style::MONO => d.text_sm().font_family("monospace"),
            _ => d.text_sm(),
        }
        .child(text)
        .into_any_element()
    }

    fn render_heading(&self, node: &Node) -> AnyElement {
        let level = node.prop(prop::LEVEL).and_then(Value::as_int).unwrap_or(2);
        let text = node.text.clone().unwrap_or_default();
        let d = aligned(div().font_weight(FontWeight::SEMIBOLD), node);
        match level {
            1 => d.text_2xl().font_weight(FontWeight::BOLD),
            2 => d.text_xl(),
            _ => d.text_lg(),
        }
        .child(text)
        .into_any_element()
    }

    fn render_button(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let label = node.str_prop(prop::LABEL).map(str::to_owned).or_else(|| node.text.clone()).unwrap_or_default();
        let disabled = node.bool_prop(prop::DISABLED) || self.stopped.is_some();
        let (bg, fg, hover) = match node.enum_prop(prop::ROLE) {
            button_role::PRIMARY => (t.accent, t.on_accent, t.accent_hover),
            button_role::DESTRUCTIVE => (t.surface_alt, t.danger, t.hover),
            _ => (t.surface_alt, t.text, t.hover),
        };
        let handler = node.handler(event::PRESS);
        let large = node.enum_prop(prop::SIZE) == button_size::LARGE;
        div()
            .id(eid("btn", node.id))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .px_4()
            .rounded_lg()
            .bg(bg)
            .text_color(fg)
            .map(|d| if large { d.h(px(56.)).text_xl() } else { d.h(px(36.)).text_sm() })
            .font_weight(FontWeight::MEDIUM)
            .child(label)
            .when(disabled, |d| d.opacity(0.5))
            .when(!disabled, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
            .when_some(handler.filter(|_| !disabled), |d, h| {
                d.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.fire(h, event::PRESS, Value::Null, cx)))
            })
            .into_any_element()
    }

    /// A labelled field. The label goes above the input on compact and beside
    /// it on regular and wide (SPEC.md §6.4).
    fn labelled(&self, label: String, input: impl IntoElement, error: Option<String>, t: &Tokens) -> AnyElement {
        let label = div().text_sm().text_color(t.text_muted).child(label);
        let input_col = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(input)
            .when_some(error, |d, e| d.child(div().text_xs().text_color(t.danger).child(e)));
        if self.class == WidthClass::Compact {
            div().flex().flex_col().gap_1().child(label).child(input_col).into_any_element()
        } else {
            div().flex().items_center().gap_4().child(label.w(px(140.)).flex_none()).child(input_col).into_any_element()
        }
    }

    fn render_text_field(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        let placeholder = node.str_prop(prop::PLACEHOLDER).unwrap_or("").to_owned();
        let error = node.str_prop(prop::ERROR).map(str::to_owned);
        let Some(field) = self.fields.get(&node.id) else { return div().into_any_element() };
        let id = node.id;
        let input = div()
            .id(eid("tf-wrap", id))
            .w_full()
            .capture_action(cx.listener(move |this, _: &Enter, _, cx| {
                let Some(node) = this.tree.get(id) else { return };
                if let Some(h) = node.handler(event::SUBMIT) {
                    let value = node.str_prop(prop::VALUE).unwrap_or("").to_owned();
                    this.fire(h, event::SUBMIT, Value::Str(value), cx);
                }
            }))
            .child(
                text_input(eid("tf", id))
                    .state(field.state.downgrade())
                    .accepts_input(self.stopped.is_none())
                    .placeholder(placeholder)
                    .placeholder_color(t.text_muted)
                    .caret_color(t.text)
                    .selection_color(t.selection)
                    .caret_blink_interval_500ms()
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(if error.is_some() { t.danger } else { t.border })
                    .bg(t.background)
                    .text_color(t.text)
                    .text_sm()
                    .min_h_auto()
                    .whitespace_nowrap()
                    .overflow_x_scroll(),
            );
        self.labelled(label, input, error, t)
    }

    fn render_toggle(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        let on = node.bool_prop(prop::VALUE);
        let disabled = node.bool_prop(prop::DISABLED) || self.stopped.is_some();
        let id = node.id;
        let switch = div()
            .id(eid("toggle", id))
            .flex_none()
            .w(px(44.))
            .h(px(26.))
            .p(px(3.))
            .rounded_full()
            .flex()
            .when(on, |d| d.justify_end())
            .bg(if on { t.accent } else { t.track })
            .child(div().size(px(20.)).rounded_full().bg(t.knob))
            .when(disabled, |d| d.opacity(0.5))
            .when(!disabled, |d| {
                d.cursor_pointer().on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let Some(node) = this.tree.get(id) else { return };
                    let next = !node.bool_prop(prop::VALUE);
                    let handler = node.handler(event::CHANGE);
                    this.tree.set_local_prop(id, prop::VALUE, Value::Bool(next));
                    match handler {
                        Some(h) => this.fire(h, event::CHANGE, Value::Bool(next), cx),
                        None => cx.notify(),
                    }
                }))
            });
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(div().text_sm().child(label))
            .child(switch)
            .into_any_element()
    }

    fn render_list(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let count = node.children.len();
        let rows = node.children.iter().enumerate().map(|(i, &c)| {
            div()
                .when(i + 1 < count, |d| d.border_b_1().border_color(t.border))
                .child(self.render_node(c, t, cx))
        });
        // Rows run edge to edge inside the section card.
        div().flex().flex_col().mx(px(-8.)).children(rows.collect::<Vec<_>>()).into_any_element()
    }

    fn render_row(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let title = node.str_prop(prop::TITLE).unwrap_or("").to_owned();
        let subtitle = node.str_prop(prop::SUBTITLE).map(str::to_owned);
        let glyph = node.str_prop(prop::ICON).map(icon_glyph);
        let handler = node.handler(event::PRESS).filter(|_| self.stopped.is_none());
        let hover = t.hover;
        div()
            .id(eid("row", node.id))
            .flex()
            .items_center()
            .gap_3()
            .px_2()
            .py_2()
            .min_h(px(44.))
            .rounded_md()
            .when_some(glyph, |d, g| d.child(div().w(px(20.)).text_color(t.accent).child(g)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_sm().child(title))
                    .when_some(subtitle, |d, s| d.child(div().text_xs().text_color(t.text_muted).child(s))),
            )
            .child(div().flex().flex_none().items_center().gap_2().children(self.render_children(node, t, cx)))
            .when_some(handler, |d, h| {
                d.cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.fire(h, event::PRESS, Value::Null, cx)))
            })
            .into_any_element()
    }

    /// A group (UI API 1.1). In a row, each child gets the same width; the
    /// runtime still owns the spacing.
    fn render_group(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let row = match node.enum_prop(prop::AXIS) {
            axis::ROW => true,
            axis::COLUMN => false,
            _ => self.class != WidthClass::Compact,
        };
        let kids = self.render_children(node, t, cx);
        if row {
            div()
                .w_full()
                .flex()
                .flex_row()
                .gap_2()
                .children(kids.into_iter().map(|k| div().flex_1().min_w_0().flex().flex_col().child(k)))
                .into_any_element()
        } else {
            div().w_full().flex().flex_col().gap_3().children(kids).into_any_element()
        }
    }

    fn render_empty(&self, node: &Node, t: &Tokens) -> AnyElement {
        let title = node.str_prop(prop::TITLE).unwrap_or("").to_owned();
        let message = node.str_prop(prop::MESSAGE).map(str::to_owned);
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .py_6()
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(title))
            .when_some(message, |d, m| d.child(div().text_xs().text_color(t.text_muted).child(m)))
            .into_any_element()
    }
}

/// Applies the `align` prop of Text and Heading (UI API 1.1).
fn aligned(d: gpui::Div, node: &Node) -> gpui::Div {
    match node.enum_prop(prop::ALIGN) {
        text_align::CENTER => d.text_center(),
        text_align::END => d.text_right(),
        _ => d,
    }
}
