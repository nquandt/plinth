//! The gpui-ce view that renders the semantic tree (SPEC.md §6 and §9.3).
//!
//! The runtime owns all layout, spacing and color. Apps only supply the tree.

use crate::theme::{Tokens, WidthClass, icon_glyph, with_alpha};
use crate::tree::{Node, Tree};
use gpui::{
    AnyElement, ClickEvent, Context, ElementId, Entity, FontWeight, IntoElement, Render,
    SharedString, Subscription, Window, div, prelude::*, px,
};
use gpui_elements::editable_text::actions::Enter;
use gpui_elements::editable_text::{EditableTextState, StringStorage, TextChanged, text_area, text_input};
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

/// Host-side state of one `TextField`, `TextArea` or `NumberField`.
struct Field {
    state: Entity<EditableTextState>,
    /// The last value that the host and the guest agree on, as text.
    last_synced: String,
    /// `NumberField` holds its value as text but the prop is a number.
    numeric: bool,
    _subscription: Subscription,
}

/// Formats a `Value::Number` the way a `NumberField` shows it: no trailing
/// `.0` for whole numbers (SPEC.md §6.3).
fn format_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v}") }
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
            let node = self.tree.get(*id);
            let guest_value = if field.numeric {
                node.and_then(|n| n.num_prop(prop::VALUE)).map(format_num).unwrap_or_default()
            } else {
                node.and_then(|n| n.str_prop(prop::VALUE)).unwrap_or("").to_owned()
            };
            if guest_value != field.last_synced {
                field.last_synced = guest_value.clone();
                field.state.update(cx, |s, cx| s.emplace(&guest_value, cx));
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
        if field.numeric {
            let Ok(mut n) = text.trim().parse::<f64>() else { return };
            if let Some(node) = self.tree.get(id) {
                if let Some(min) = node.num_prop(prop::MIN) {
                    n = n.max(min);
                }
                if let Some(max) = node.num_prop(prop::MAX) {
                    n = n.min(max);
                }
            }
            self.tree.set_local_prop(id, prop::VALUE, Value::Number(n));
            if let Some(handler) = self.tree.get(id).and_then(|n| n.handler(event::CHANGE)) {
                self.fire(handler, event::CHANGE, Value::Number(n), cx);
            }
        } else {
            self.tree.set_local_prop(id, prop::VALUE, Value::Str(text.clone()));
            if let Some(handler) = self.tree.get(id).and_then(|n| n.handler(event::CHANGE)) {
                self.fire(handler, event::CHANGE, Value::Str(text), cx);
            }
        }
    }

    /// Makes the text field states for visible `TextField`, `TextArea` and
    /// `NumberField` nodes. This must happen before the element pass,
    /// because it needs `&mut self`.
    fn ensure_fields(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.tree.current_root() else { return };
        let mut stack = vec![root.id];
        let mut missing = Vec::new();
        while let Some(id) = stack.pop() {
            let Some(node) = self.tree.get(id) else { continue };
            let numeric = node.kind == Some(ControlKind::NumberField);
            let is_text_input = matches!(node.kind, Some(ControlKind::TextField) | Some(ControlKind::TextArea)) || numeric;
            if is_text_input && !self.fields.contains_key(&id) {
                let initial =
                    if numeric { node.num_prop(prop::VALUE).map(format_num).unwrap_or_default() } else { node.str_prop(prop::VALUE).unwrap_or("").to_owned() };
                missing.push((id, initial, numeric));
            }
            stack.extend(node.children.iter().copied());
        }
        for (id, value, numeric) in missing {
            let initial = value.clone();
            let state = cx.new(|cx| EditableTextState::new(StringStorage::from(initial), cx));
            let subscription =
                cx.subscribe(&state, move |this, state, _: &TextChanged, cx| this.on_text_changed(id, state, cx));
            self.fields.insert(id, Field { state, last_synced: value, numeric, _subscription: subscription });
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
            ControlKind::Checkbox => self.render_checkbox(node, t, cx),
            ControlKind::TextArea => self.render_text_area(node, t, cx),
            ControlKind::Slider => self.render_slider(node, t, cx),
            ControlKind::NumberField => self.render_number_field(node, t, cx),
            ControlKind::Picker => self.render_picker(node, t, cx),
            ControlKind::Progress => self.render_progress(node, t),
            ControlKind::Badge => self.render_badge(node, t),
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

    // -- UI API 1.2 inputs --

    fn render_checkbox(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        let checked = node.bool_prop(prop::VALUE);
        let disabled = node.bool_prop(prop::DISABLED) || self.stopped.is_some();
        let id = node.id;
        let box_el = div()
            .id(eid("checkbox", id))
            .flex_none()
            .size(px(18.))
            .rounded_sm()
            .border_1()
            .border_color(if checked { t.accent } else { t.border })
            .bg(if checked { t.accent } else { t.background })
            .flex()
            .items_center()
            .justify_center()
            .when(checked, |d| d.child(div().text_xs().text_color(t.knob).child("✓")))
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
        div().flex().items_center().gap_2().child(box_el).child(div().text_sm().text_color(t.text).child(label)).into_any_element()
    }

    fn render_text_area(&self, node: &Node, t: &Tokens, _cx: &mut Context<Self>) -> AnyElement {
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        let placeholder = node.str_prop(prop::PLACEHOLDER).unwrap_or("").to_owned();
        let Some(field) = self.fields.get(&node.id) else { return div().into_any_element() };
        let id = node.id;
        let input = div().id(eid("ta-wrap", id)).w_full().child(
            text_area(eid("ta", id))
                .state(field.state.downgrade())
                .accepts_input(self.stopped.is_none())
                .placeholder(placeholder)
                .placeholder_color(t.text_muted)
                .caret_color(t.text)
                .selection_color(t.selection)
                .caret_blink_interval_500ms()
                .w_full()
                .min_h(px(80.))
                .px_3()
                .py_2()
                .rounded_lg()
                .border_1()
                .border_color(t.border)
                .bg(t.background)
                .text_color(t.text)
                .text_sm(),
        );
        self.labelled(label, input, None, t)
    }

    /// Steps a `Slider` or `NumberField` value by `delta`, clamped to
    /// `min`/`max`, and fires `onChange` if the app set one.
    fn step_value(&mut self, id: NodeId, delta: f64, cx: &mut Context<Self>) {
        let Some(node) = self.tree.get(id) else { return };
        let min = node.num_prop(prop::MIN).unwrap_or(f64::MIN);
        let max = node.num_prop(prop::MAX).unwrap_or(f64::MAX);
        let current = node.num_prop(prop::VALUE).unwrap_or(0.0);
        let next = (current + delta).clamp(min, max);
        let handler = node.handler(event::CHANGE);
        self.tree.set_local_prop(id, prop::VALUE, Value::Number(next));
        match handler {
            Some(h) => self.fire(h, event::CHANGE, Value::Number(next), cx),
            None => cx.notify(),
        }
    }

    fn step_button(&self, id: NodeId, label: &'static str, delta: f64, disabled: bool, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(eid(if delta < 0.0 { "step-down" } else { "step-up" }, id))
            .flex_none()
            .size(px(28.))
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.surface_alt)
            .text_color(t.text)
            .when(disabled, |d| d.opacity(0.5))
            .when(!disabled, |d| {
                d.cursor_pointer().hover(move |s| s.bg(t.hover)).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.step_value(id, delta, cx);
                }))
            })
            .child(label)
            .into_any_element()
    }

    fn render_slider(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        let min = node.num_prop(prop::MIN).unwrap_or(0.0);
        let max = node.num_prop(prop::MAX).unwrap_or(1.0);
        let step = node.num_prop(prop::STEP).unwrap_or(((max - min) / 20.0).max(0.000_1));
        let value = node.num_prop(prop::VALUE).unwrap_or(min).clamp(min, max);
        let disabled = node.bool_prop(prop::DISABLED) || self.stopped.is_some();
        let id = node.id;
        let frac = if max > min { ((value - min) / (max - min)).clamp(0.0, 1.0) } else { 0.0 };
        let track = div()
            .flex_1()
            .h(px(8.))
            .rounded_full()
            .bg(t.track)
            .child(div().h_full().rounded_full().bg(t.accent).w(gpui::relative(frac as f32)));
        let row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(self.step_button(id, "-", -step, disabled, t, cx))
            .child(track)
            .child(self.step_button(id, "+", step, disabled, t, cx))
            .child(div().w(px(44.)).text_xs().text_color(t.text_muted).text_right().child(format_num(value)));
        self.labelled(label, row, None, t)
    }

    fn render_number_field(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        let step = node.num_prop(prop::STEP).unwrap_or(1.0);
        let disabled = node.bool_prop(prop::DISABLED) || self.stopped.is_some();
        let id = node.id;
        let Some(field) = self.fields.get(&id) else { return div().into_any_element() };
        let input = div().id(eid("nf-wrap", id)).flex().items_center().gap_1().child(self.step_button(id, "-", -step, disabled, t, cx)).child(
            text_input(eid("nf", id))
                .state(field.state.downgrade())
                .accepts_input(self.stopped.is_none())
                .caret_color(t.text)
                .selection_color(t.selection)
                .caret_blink_interval_500ms()
                .w(px(64.))
                .px_2()
                .py_2()
                .rounded_lg()
                .border_1()
                .border_color(t.border)
                .bg(t.background)
                .text_color(t.text)
                .text_sm()
                .text_center()
                .min_h_auto(),
        ).child(self.step_button(id, "+", step, disabled, t, cx));
        self.labelled(label, input, None, t)
    }

    /// A `Picker` (SPEC.md §6.3): a segmented control for up to 4 options,
    /// or a stacked list of rows for more.
    fn render_picker(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        let current = node.str_prop(prop::VALUE).unwrap_or("").to_owned();
        let disabled = node.bool_prop(prop::DISABLED) || self.stopped.is_some();
        let id = node.id;
        let options: Vec<String> = node.str_prop(prop::OPTIONS).unwrap_or("").split('\u{1f}').filter(|s| !s.is_empty()).map(str::to_owned).collect();
        let select = move |this: &mut Self, opt: String, cx: &mut Context<Self>| {
            let handler = this.tree.get(id).and_then(|n| n.handler(event::CHANGE));
            this.tree.set_local_prop(id, prop::VALUE, Value::Str(opt.clone()));
            match handler {
                Some(h) => this.fire(h, event::CHANGE, Value::Str(opt), cx),
                None => cx.notify(),
            }
        };
        let body = if options.len() <= 4 {
            let items = options.into_iter().enumerate().map(|(i, opt)| {
                let selected = opt == current;
                let opt_for_click = opt.clone();
                div()
                    .id(eid("picker-opt", (id as u64) * 100 + i as u64))
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .py_1()
                    .rounded_md()
                    .text_xs()
                    .when(selected, |d| d.bg(t.accent).text_color(t.on_accent))
                    .when(!selected, |d| d.text_color(t.text))
                    .when(disabled, |d| d.opacity(0.5))
                    .when(!disabled, |d| {
                        d.cursor_pointer().on_click(cx.listener(move |this, _: &ClickEvent, _, cx| select(this, opt_for_click.clone(), cx)))
                    })
                    .child(opt)
            });
            div().flex().gap_1().p_1().rounded_lg().bg(t.surface_alt).children(items).into_any_element()
        } else {
            let hover = t.hover;
            let rows = options.into_iter().map(|opt| {
                let selected = opt == current;
                let opt_for_click = opt.clone();
                div()
                    .id(eid("picker-row", {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        opt.hash(&mut h);
                        h.finish()
                    }))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .when(disabled, |d| d.opacity(0.5))
                    .when(!disabled, |d| {
                        d.cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| select(this, opt_for_click.clone(), cx)))
                    })
                    .child(div().text_sm().child(opt))
                    .when(selected, |d| d.child(div().text_color(t.accent).child("✓")))
            });
            div().flex().flex_col().rounded_lg().border_1().border_color(t.border).children(rows).into_any_element()
        };
        self.labelled(label, body, None, t)
    }

    fn render_progress(&self, node: &Node, t: &Tokens) -> AnyElement {
        let label = node.str_prop(prop::LABEL).map(str::to_owned);
        let value = node.num_prop(prop::VALUE).map(|v| v.clamp(0.0, 1.0) as f32);
        let bar = div().w_full().h(px(6.)).rounded_full().bg(t.track).child(match value {
            Some(frac) => div().h_full().rounded_full().bg(t.accent).w(gpui::relative(frac)).into_any_element(),
            // Indeterminate: a fixed-width segment. There is no animation yet.
            None => div().h_full().rounded_full().bg(t.accent).w(gpui::relative(0.3)).into_any_element(),
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .when_some(label, |d, l| d.child(div().text_sm().text_color(t.text_muted).child(l)))
            .child(bar)
            .into_any_element()
    }

    fn render_badge(&self, node: &Node, t: &Tokens) -> AnyElement {
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        let (bg, fg) = match node.enum_prop(prop::TONE) {
            tone::DANGER => (with_alpha(t.danger, 0.16), t.danger),
            tone::SUCCESS => (with_alpha(t.success, 0.16), t.success),
            tone::MUTED => (t.surface_alt, t.text_muted),
            _ => (with_alpha(t.accent, 0.16), t.accent),
        };
        div()
            .flex_none()
            .px_2()
            .py_0p5()
            .rounded_full()
            .bg(bg)
            .text_color(fg)
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .child(label)
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
