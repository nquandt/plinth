//! The gpui-ce view that renders the semantic tree (SPEC.md §6 and §9.3).
//!
//! The runtime owns all layout, spacing and color. Apps only supply the tree.

use crate::theme::{Tokens, WidthClass, icon_glyph, with_alpha};
use crate::tree::{Node, Tree};
use gpui::{
    AnyElement, Bounds, ClickEvent, Context, DragMoveEvent, ElementId, Entity, FocusHandle, FontWeight, Image, ImageFormat,
    DispatchPhase, IntoElement, KeyDownEvent, KeyUpEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels,
    Point, Render, SharedString, Stateful, Subscription, Window, anchored, canvas, deferred, div, img, prelude::*, px,
};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use gpui::accesskit;
use gpui_elements::editable_text::actions::Enter;
use gpui_elements::editable_text::{EditableTextState, StringStorage, TextChanged, text_area, text_input};
use crate::calendar;
use crate::primitives;
use plinth_protocol::{
    ControlKind, Event, NodeId, Op, Value, Writer, aspect, axis, button_role, button_size, chart_kind, date_picker_mode,
    cross_align, decode_ops, event, justify, lifecycle_kind, pressable_role, prop, text_align, text_style, tone,
};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// A `List` with more rows than this renders as a virtual list (SPEC.md
/// §7.3): only the rows near the viewport become elements, so a list of
/// thousands of rows no longer costs one element per row per frame.
const VIRTUAL_LIST_THRESHOLD: usize = 200;

/// A column `Scroll` with this many children or more, each with a fixed
/// height, builds only the children near its viewport.
const CULL_MIN_CHILDREN: usize = 40;
/// A row in a horizontal `Scroll` with this many fixed-width children or
/// more builds only the ones in view (a short row of cards builds all).
const CULL_MIN_ROW: usize = 16;

/// Spacing units as pixels, for the culling arithmetic.
fn px_of_units(units: f64) -> f32 {
    units.clamp(0.0, primitives::MAX_UNITS as f64) as f32 * primitives::UNIT
}

/// The width of fixed-width children in a row with `gap` between them.
fn total_extent(widths: &[f32], gap: f32) -> f32 {
    widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32
}

/// The x range for the children of a column box: they start at the left
/// padding when they stretch or align to the start; else no range.
fn column_child_range(range: Option<(f32, f32)>, style: &primitives::Style) -> Option<(f32, f32)> {
    let (lo, hi) = range?;
    if !matches!(style.enum_(prop::CROSS_ALIGN), cross_align::STRETCH | cross_align::START) {
        return None;
    }
    let pad_left = style.get_f(prop::PADDING_X).or_else(|| style.get_f(prop::PADDING)).map_or(0.0, px_of_units);
    Some((lo - pad_left, hi - pad_left))
}
/// The extra height above and below the viewport that is built too, in pixels.
const CULL_OVERSCAN: f32 = 200.0;
/// The viewport height to assume before the first frame, in pixels.
const CULL_FIRST_FRAME: f32 = 1200.0;

/// The host side of the guest connection. `dispatch` sends one event buffer
/// and returns the op buffers that the guest committed while it ran.
pub trait GuestPort {
    fn dispatch(&mut self, events: &[u8]) -> anyhow::Result<Vec<Vec<u8>>>;

    /// The soonest time a `plinth:time` timer is due, if any is pending
    /// (SPEC.md §8.5). The default (no timers) suits a guest with no host
    /// API access.
    fn next_timer_deadline(&self) -> Option<Instant> {
        None
    }

    /// Dispatches a `timer` event for every timer due at or before `now`
    /// and returns the op buffers the guest committed. The default does
    /// nothing.
    fn fire_due_timers(&mut self, now: Instant) -> anyhow::Result<Vec<Vec<u8>>> {
        let _ = now;
        Ok(Vec::new())
    }

    /// Whether the guest has a frame timer (core 1.12, `onFrame`): the host
    /// then draws frames without a pause. The default has none.
    fn wants_frames(&self) -> bool {
        false
    }

    /// Dispatches a `frame` event to each frame timer and returns the op
    /// buffers the guest committed. The default does nothing.
    fn fire_frame(&mut self, now: Instant) -> anyhow::Result<Vec<Vec<u8>>> {
        let _ = now;
        Ok(Vec::new())
    }

    /// `plinth:dialog` requests the guest opened that no answer has closed
    /// yet (SPEC.md §8.4, §8.5). The default (no dialog host API access)
    /// has none.
    fn pending_dialogs(&self) -> Vec<PendingDialog> {
        Vec::new()
    }

    /// Answers a dialog request by id, delivering a `completion` event to
    /// the guest and returning the op buffers it committed. The default is
    /// a no-op (there is nothing to answer).
    fn answer_dialog(&mut self, id: u32, value: Value) -> anyhow::Result<Vec<Vec<u8>>> {
        let _ = (id, value);
        Ok(Vec::new())
    }

    /// Drains `plinth:net` requests a worker thread finished since the
    /// last poll (SPEC.md §8.4, §8.5): `(request id, result)` pairs ready
    /// for `answer_dialog` (the same generic completion delivery dialogs
    /// use). The default (no net host API access) has none.
    fn poll_net_results(&mut self) -> Vec<(u32, Value)> {
        Vec::new()
    }

    /// The uncaught app errors (`error.report`, SPEC.md §5.6) since the
    /// last call. The app is still running. The default has none.
    fn take_errors(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Drains the app ids that a privileged Hub UI guest asked the host to
    /// launch (`plinth:hub`'s `launch`, `docs/HUB.md` §4.1, §4.2). The
    /// default (no `plinth:hub` backend) has none.
    fn take_hub_launches(&mut self) -> Vec<String> {
        Vec::new()
    }
}

/// Which `plinth:dialog` call opened a pending dialog request (mirrors
/// `plinth_runner_wasmtime::DialogKind` without a dependency on that crate).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
}

/// A dialog request the guest opened that the host has not answered yet.
#[derive(Clone, Debug)]
pub struct PendingDialog {
    pub id: u32,
    pub kind: DialogKind,
    pub message: String,
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

/// The gpui-ce image format for an asset's file extension, or `None` for
/// an extension the renderer does not know how to decode.
fn image_format(path: &str) -> Option<ImageFormat> {
    match path.rsplit('.').next()?.to_ascii_lowercase().as_str() {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "webp" => Some(ImageFormat::Webp),
        "svg" => Some(ImageFormat::Svg),
        "gif" => Some(ImageFormat::Gif),
        _ => None,
    }
}

/// Snaps a raw `Slider` value to the nearest multiple of `step` from `min`,
/// clamped to `[min, max]` (SPEC.md §8.4).
fn snap_slider_value(raw: f64, min: f64, max: f64, step: f64) -> f64 {
    let snapped = if step > 0.0 { min + ((raw - min) / step).round() * step } else { raw };
    snapped.clamp(min, max)
}

#[cfg(test)]
mod slider_snap_tests {
    use super::snap_slider_value;

    #[test]
    fn snaps_to_nearest_step() {
        assert_eq!(snap_slider_value(23.0, 0.0, 100.0, 10.0), 20.0);
        assert_eq!(snap_slider_value(27.0, 0.0, 100.0, 10.0), 30.0);
    }

    #[test]
    fn clamps_to_min_and_max() {
        assert_eq!(snap_slider_value(-50.0, 0.0, 100.0, 10.0), 0.0);
        assert_eq!(snap_slider_value(500.0, 0.0, 100.0, 10.0), 100.0);
    }

    #[test]
    fn zero_step_passes_through() {
        assert_eq!(snap_slider_value(42.3, 0.0, 100.0, 0.0), 42.3);
    }
}

/// Drag payload for a `Slider` track being dragged (SPEC.md §8.4). `gpui-ce`
/// identifies an in-progress drag by this entity, so each slider carries its
/// own node id to tell its drag apart from any other slider's.
#[derive(Clone)]
struct SliderDrag {
    id: NodeId,
}

impl Render for SliderDrag {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// A destructive action waiting for the user to confirm it (SPEC.md §6.4,
/// UI API 1.2).
struct PendingConfirm {
    label: String,
    handler: Option<u32>,
}

/// A `plinth:dialog` request shown as a modal overlay (SPEC.md §8.4, §8.5).
/// At most one is shown at a time; `sync_dialog` picks up the next one once
/// this one is answered.
struct ActiveDialog {
    id: u32,
    kind: DialogKind,
    message: String,
    /// The text input state for a `prompt` dialog.
    input: Option<Entity<EditableTextState>>,
}

pub struct PlinthRoot {
    tree: Tree,
    guest: Box<dyn GuestPort>,
    accent: SharedString,
    fields: HashMap<NodeId, Field>,
    /// Set when the guest traps. The tree then stays read-only.
    stopped: Option<String>,
    /// Uncaught app errors that the user did not dismiss yet (SPEC.md
    /// §5.6). The app keeps running; the host shows the last one.
    app_errors: Vec<String>,
    class: WidthClass,
    /// Toolbar overflow menus and `<Menu>` controls that are open, by node
    /// id (UI API 1.2).
    open_menus: HashSet<NodeId>,
    /// The destructive action the host is confirming, if any.
    confirm: Option<PendingConfirm>,
    /// The `plinth:dialog` request shown right now, if any.
    dialog: Option<ActiveDialog>,
    /// Holds keyboard focus so `on_key_down` (Escape to close a menu or go
    /// back) fires without depending on a focusable child being focused.
    focus: FocusHandle,
    focused_once: bool,
    /// The node that got the last pointer-down (UI API 1.12): it gets the
    /// moves outside its bounds and the pointer-up, until the button goes up.
    pointer_owner: Rc<Cell<Option<u32>>>,
    /// The last foreground state sent to the guest (`None` before the
    /// first), and the window activation observer that sends it.
    active: Option<bool>,
    activation: Option<Subscription>,
    /// The package's assets (SPEC.md §10.1), by path under `assets/`
    /// (without the prefix), for `<Image>`.
    assets: Arc<HashMap<String, Vec<u8>>>,
    /// Open `DatePicker` popovers: the displayed `(year, month)` and the
    /// keyboard-focused day of the month grid (UI API 1.4, SPEC.md §6.3).
    date_cursor: HashMap<NodeId, (i32, u32, u32)>,
    /// One `gpui::ListState` per virtual `List` node (SPEC.md §7.3), keyed
    /// by node id. `render_list` takes `&self`, so this needs a `RefCell`;
    /// entries are dropped in `apply_commits` when the node is removed.
    list_states: std::cell::RefCell<HashMap<NodeId, gpui::ListState>>,
    /// The scroll position of each Level 2 `Scroll`, by node id: the
    /// renderer builds only the children near the viewport (`culled_scroll_children`).
    scroll_handles: std::cell::RefCell<HashMap<NodeId, gpui::ScrollHandle>>,
    /// While a box's children are built: the x range, in pixels from the
    /// box's left edge, that a horizontal `Scroll` above it shows (with
    /// overscan). A row box with fixed-width children builds only the ones
    /// in it. `None`: build all (docs/GAPS.md 7G-12).
    cull_x: Cell<Option<(f32, f32)>>,
}

impl PlinthRoot {
    /// Makes the view. `initial_commits` are the op buffers from `init`.
    pub fn new(
        guest: Box<dyn GuestPort>,
        initial_commits: Vec<Vec<u8>>,
        accent: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_assets(guest, initial_commits, accent, Arc::new(HashMap::new()), cx)
    }

    /// Like [`Self::new`], with the package's assets for `<Image>`.
    pub fn with_assets(
        guest: Box<dyn GuestPort>,
        initial_commits: Vec<Vec<u8>>,
        accent: impl Into<SharedString>,
        assets: Arc<HashMap<String, Vec<u8>>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut root = Self {
            tree: Tree::new(),
            guest,
            accent: accent.into(),
            fields: HashMap::new(),
            stopped: None,
            app_errors: Vec::new(),
            class: WidthClass::Wide,
            open_menus: HashSet::new(),
            confirm: None,
            dialog: None,
            focus: cx.focus_handle(),
            focused_once: false,
            pointer_owner: Rc::new(Cell::new(None)),
            active: None,
            activation: None,
            assets,
            date_cursor: HashMap::new(),
            list_states: std::cell::RefCell::new(HashMap::new()),
            scroll_handles: std::cell::RefCell::new(HashMap::new()),
            cull_x: Cell::new(None),
        };
        root.apply_commits(initial_commits);
        root.sync_dialog(cx);
        root
    }

    /// Makes the view for a guest that trapped in `init`.
    pub fn stopped(guest: Box<dyn GuestPort>, error: String, accent: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            tree: Tree::new(),
            guest,
            accent: accent.into(),
            fields: HashMap::new(),
            stopped: Some(error),
            app_errors: Vec::new(),
            class: WidthClass::Wide,
            open_menus: HashSet::new(),
            confirm: None,
            dialog: None,
            focus: cx.focus_handle(),
            focused_once: false,
            pointer_owner: Rc::new(Cell::new(None)),
            active: None,
            activation: None,
            assets: Arc::new(HashMap::new()),
            date_cursor: HashMap::new(),
            list_states: std::cell::RefCell::new(HashMap::new()),
            scroll_handles: std::cell::RefCell::new(HashMap::new()),
            cull_x: Cell::new(None),
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

    /// Replaces the guest with a new build (hot reload, SPEC.md §13). Before
    /// swapping, it asks the old guest for a snapshot of its module-level
    /// signals (a dev build answers with an `Op::Snapshot`; a release build,
    /// or a guest that already stopped, gives nothing and the new instance
    /// just starts fresh) and passes the bytes to `make` as `init`'s `args`.
    /// The selected screen and the navigation stack stay, if the screens
    /// still exist.
    pub fn reload(
        &mut self,
        make: impl FnOnce(&[u8]) -> (Box<dyn GuestPort>, Result<Vec<Vec<u8>>, String>),
        cx: &mut Context<Self>,
    ) {
        let screen = self.tree.current_screen;
        let stacks = self.tree.stacks_snapshot();
        let snapshot = self.request_snapshot();
        let (guest, init) = make(&snapshot);
        self.guest = guest;
        self.tree = Tree::new();
        self.fields.clear();
        self.stopped = None;
        self.open_menus.clear();
        self.confirm = None;
        self.dialog = None;
        match init {
            Ok(commits) => self.apply_commits(commits),
            Err(e) => self.stopped = Some(e),
        }
        if self.tree.screens().any(|(s, _)| s == screen) {
            self.tree.current_screen = screen;
        }
        // SPEC.md §13: the selected screen and the navigation stack survive
        // a reload when the screens still exist.
        self.tree.restore_stacks(stacks);
        self.sync_dialog(cx);
        cx.notify();
    }

    /// Dev-only (SPEC.md §13): asks the current guest for a hot-reload
    /// snapshot and returns its bytes, or an empty buffer if it did not
    /// answer (a release build, or a guest that already stopped).
    fn request_snapshot(&mut self) -> Vec<u8> {
        if self.stopped.is_some() {
            return Vec::new();
        }
        let mut w = Writer::new();
        w.event(&Event::SnapshotRequest);
        let Ok(commits) = self.guest.dispatch(w.as_bytes()) else { return Vec::new() };
        for commit in commits {
            if let Ok(ops) = decode_ops(&commit) {
                for op in ops {
                    if let Op::Snapshot { bytes } = op {
                        return bytes;
                    }
                }
            }
        }
        Vec::new()
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
            self.list_states.borrow_mut().remove(&id);
            self.scroll_handles.borrow_mut().remove(&id);
        }
        self.app_errors.extend(self.guest.take_errors());
    }

    /// Tells the guest that its window went to the foreground or the
    /// background (the `lifecycle` event, core 1.12 `isActive`).
    fn send_lifecycle(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.stopped.is_some() || self.active == Some(active) {
            return;
        }
        self.active = Some(active);
        let kind = if active { lifecycle_kind::FOREGROUND } else { lifecycle_kind::BACKGROUND };
        let mut w = Writer::new();
        w.event(&Event::Lifecycle { kind });
        match self.guest.dispatch(w.as_bytes()) {
            Ok(commits) => self.apply_commits(commits),
            Err(e) => {
                log::error!("the app stopped: {e:#}");
                self.stopped = Some(format!("{e}"));
            }
        }
        self.sync_fields(cx);
        self.sync_dialog(cx);
        cx.notify();
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
                self.stopped = Some(format!("{e}"));
            }
        }
        self.sync_fields(cx);
        self.sync_dialog(cx);
        cx.notify();
    }

    /// The soonest time the guest has a `plinth:time` timer due, if any.
    /// A host drives timers by sleeping until this instant (at most a
    /// short, fixed interval, for net results) and calling `poll_timers`.
    pub fn next_timer_deadline(&self) -> Option<Instant> {
        self.guest.next_timer_deadline()
    }

    /// Dispatches every timer due by now and applies what the guest
    /// commits (SPEC.md §8.4, §8.5), mirroring `fire` for `ui` events.
    /// Also delivers any `plinth:net` result a worker thread finished
    /// (SPEC.md §8.4, §8.5): `net.fetch` has no deadline of its own, so the
    /// host also polls at a fixed, longest interval.
    pub fn poll_timers(&mut self, cx: &mut Context<Self>) {
        if self.stopped.is_some() {
            return;
        }
        let mut changed = false;
        match self.guest.fire_due_timers(Instant::now()) {
            Ok(commits) if !commits.is_empty() => {
                self.apply_commits(commits);
                changed = true;
            }
            Ok(_) => {}
            Err(e) => {
                log::error!("the app stopped: {e:#}");
                self.stopped = Some(format!("{e}"));
                return;
            }
        }
        for (id, result) in self.guest.poll_net_results() {
            match self.guest.answer_dialog(id, result) {
                Ok(commits) => {
                    self.apply_commits(commits);
                    changed = true;
                }
                Err(e) => {
                    log::error!("the app stopped: {e:#}");
                    self.stopped = Some(format!("{e}"));
                    return;
                }
            }
        }
        // A timer or a result can start a frame timer (`onFrame`) with no
        // change to the tree: draw a frame so that the frames start.
        if !changed && !self.guest.wants_frames() {
            return;
        }
        self.sync_fields(cx);
        self.sync_dialog(cx);
        cx.notify();
    }

    /// Fires the guest's frame timers (core 1.12, `onFrame`) before a frame
    /// is drawn, and asks gpui for the next frame while a frame timer runs.
    /// gpui draws no frames for a hidden window, so the frames stop too.
    fn drive_frames(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stopped.is_some() || !self.guest.wants_frames() {
            return;
        }
        match self.guest.fire_frame(Instant::now()) {
            Ok(commits) => {
                if !commits.is_empty() {
                    self.apply_commits(commits);
                    self.sync_fields(cx);
                    self.sync_dialog(cx);
                }
            }
            Err(e) => {
                log::error!("the app stopped: {e:#}");
                self.stopped = Some(format!("{e}"));
                return;
            }
        }
        if self.guest.wants_frames() {
            window.request_animation_frame();
        }
    }

    /// Drains the launch requests of a Hub UI guest (`docs/HUB.md` §4.2).
    /// The host polls this and opens a window for each app id.
    pub fn take_hub_launches(&mut self) -> Vec<String> {
        self.guest.take_hub_launches()
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

    /// Shows the next unanswered `plinth:dialog` request as a modal
    /// overlay, if none is already shown (SPEC.md §8.4, §8.5). Called
    /// after every guest call that could have opened one.
    fn sync_dialog(&mut self, cx: &mut Context<Self>) {
        if self.dialog.is_some() {
            return;
        }
        let Some(next) = self.guest.pending_dialogs().into_iter().next() else { return };
        let input = (next.kind == DialogKind::Prompt)
            .then(|| cx.new(|cx| EditableTextState::new(StringStorage::from(String::new()), cx)));
        self.dialog = Some(ActiveDialog { id: next.id, kind: next.kind, message: next.message, input });
    }

    /// Answers the dialog shown right now with `value`, applies what the
    /// guest commits, and shows the next pending dialog, if any.
    fn answer_dialog(&mut self, value: Value, cx: &mut Context<Self>) {
        let Some(dialog) = self.dialog.take() else { return };
        match self.guest.answer_dialog(dialog.id, value) {
            Ok(commits) => self.apply_commits(commits),
            Err(e) => {
                log::error!("the app stopped: {e:#}");
                self.stopped = Some(format!("{e}"));
            }
        }
        self.sync_dialog(cx);
        cx.notify();
    }

    /// The standard modal for a `plinth:dialog` request (SPEC.md §8.4,
    /// §8.5), styled like `render_confirm_overlay`.
    fn render_host_dialog_overlay(&self, t: &Tokens, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.dialog.as_ref()?;
        let message = dialog.message.clone();
        let kind = dialog.kind;
        let input_field = dialog.input.clone();
        let input_row = input_field.clone().map(|state| {
            text_input(eid("dlg", dialog.id))
                .state(state.downgrade())
                .accepts_input(true)
                .caret_color(t.text)
                .selection_color(t.selection)
                .caret_blink_interval_500ms()
                .w_full()
                .px_3()
                .py_2()
                .rounded_lg()
                .border_1()
                .border_color(t.border)
                .bg(t.background)
                .text_color(t.text)
                .text_sm()
                .min_h_auto()
        });

        let mut buttons = div().flex().justify_end().gap_2();
        if kind != DialogKind::Alert {
            buttons = buttons.child(
                div()
                    .id("dialog-cancel")
                    .cursor_pointer()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .text_sm()
                    .child("Cancel")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        let cancel = match this.dialog.as_ref().map(|d| d.kind) {
                            Some(DialogKind::Confirm) => Value::Bool(false),
                            _ => Value::Null,
                        };
                        this.answer_dialog(cancel, cx);
                    })),
            );
        }
        buttons = buttons.child(
            div()
                .id("dialog-ok")
                .cursor_pointer()
                .px_3()
                .py_2()
                .rounded_md()
                .bg(t.accent)
                .text_color(t.on_accent)
                .text_sm()
                .child("OK")
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let value = match this.dialog.as_ref() {
                        Some(d) if d.kind == DialogKind::Confirm => Value::Bool(true),
                        Some(d) if d.kind == DialogKind::Prompt => {
                            let text = d.input.as_ref().map(|s| s.read(cx).as_str().to_owned()).unwrap_or_default();
                            Value::Str(text)
                        }
                        _ => Value::Null,
                    };
                    this.answer_dialog(value, cx);
                })),
        );

        let panel = div()
            .id("dialog-panel")
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.stop_propagation()))
            .role(accesskit::Role::Dialog)
            .aria_label(message.clone())
            .w(px(320.))
            .p_5()
            .flex()
            .flex_col()
            .gap_4()
            .rounded_xl()
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .child(div().text_sm().text_color(t.text).child(message))
            .children(input_row)
            .child(buttons);
        Some(
            div()
                .id("dialog-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(t.backdrop)
                .child(panel)
                .into_any_element(),
        )
    }
}

fn eid(prefix: &'static str, id: impl Into<u64>) -> ElementId {
    ElementId::NamedInteger(prefix.into(), id.into())
}

// -- Chart (SPEC.md §6.3, UI API 1.5) ----------------------------------------

use crate::chart::ChartPoints;

/// Decodes `data`/`series` (wire format in `wit/plinth/ui-api.toml`) into
/// `(series name, points)`. A single-series `data` gets one entry with an
/// empty name. Malformed numbers become `0.0` rather than panicking: a
/// drawing bug should never crash the host.
fn chart_series(node: &Node) -> Vec<(String, ChartPoints)> {
    if let Some(s) = node.str_prop(prop::SERIES).filter(|s| !s.is_empty()) {
        return s.split('\u{1e}').map(|one| {
            let (name, points) = one.split_once('\u{1}').unwrap_or((one, ""));
            (name.to_string(), parse_points(points))
        }).collect();
    }
    let pts = parse_points(node.str_prop(prop::DATA).unwrap_or(""));
    if pts.is_empty() { Vec::new() } else { vec![(String::new(), pts)] }
}

fn parse_points(s: &str) -> ChartPoints {
    if s.is_empty() {
        return Vec::new();
    }
    s.split('\u{1f}')
        .filter_map(|pair| {
            let (label, value) = pair.split_once('\u{1}')?;
            Some((label.to_string(), value.parse::<f64>().unwrap_or(0.0)))
        })
        .collect()
}

/// The AccessKit summary of every value (there is no visual data table).
fn chart_description(series: &[(String, ChartPoints)]) -> String {
    series
        .iter()
        .map(|(name, pts)| {
            let body = pts.iter().map(|(l, v)| format!("{l}: {v}")).collect::<Vec<_>>().join(", ");
            if name.is_empty() { body } else { format!("{name}: {body}") }
        })
        .collect::<Vec<_>>()
        .join(". ")
}

/// A row of color-swatch legend entries.
fn render_legend(names: &[String], t: &Tokens) -> AnyElement {
    let palette = t.chart_palette();
    let entries = names.iter().enumerate().map(|(i, name)| {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(div().w(px(10.)).h(px(10.)).rounded_sm().bg(palette[i % palette.len()]))
            .child(div().text_xs().text_color(t.text_muted).child(name.clone()))
            .into_any_element()
    });
    div().flex().flex_wrap().gap_3().children(entries).into_any_element()
}

#[cfg(test)]
mod chart_wire_tests {
    use super::{chart_description, parse_points};

    #[test]
    fn parses_points_and_skips_malformed() {
        let pts = parse_points("Jan\u{1}10\u{1f}Feb\u{1}12.5");
        assert_eq!(pts, vec![("Jan".to_string(), 10.0), ("Feb".to_string(), 12.5)]);
        assert_eq!(parse_points(""), Vec::<(String, f64)>::new());
    }

    #[test]
    fn describes_every_value() {
        let series = vec![("".to_string(), parse_points("Jan\u{1}10\u{1f}Feb\u{1}12.5"))];
        assert_eq!(chart_description(&series), "Jan: 10, Feb: 12.5");
    }

    #[test]
    fn describes_named_series() {
        let series = vec![
            ("2025".to_string(), parse_points("Jan\u{1}10")),
            ("2026".to_string(), parse_points("Jan\u{1}12")),
        ];
        assert_eq!(chart_description(&series), "2025: Jan: 10. 2026: Jan: 12");
    }
}

/// Warns once per render that an interactive node has no label (a cheap hub
/// lint, SPEC.md §12 item 5). Screen readers announce such a node with no
/// name, which makes it useless to a non-sighted user.
fn warn_if_unlabeled(control: &'static str, id: NodeId, label: &str) {
    if label.trim().is_empty() {
        log::warn!("a11y: {control} #{id} has no label; assistive technology cannot announce it");
    }
}

/// Adds keyboard reachability (Tab order) and activation (Enter/Space) to a
/// stateful element that already has an `on_click` handler, so pointer and
/// keyboard input drive the same code path (SPEC.md §6.1 item 3, §9.3).
fn keyboard_activatable<E>(
    d: Stateful<E>,
    cx: &mut Context<PlinthRoot>,
    on_activate: impl Fn(&mut PlinthRoot, &mut Context<PlinthRoot>) + 'static,
) -> Stateful<E>
where
    E: IntoElement + 'static,
    Stateful<E>: InteractiveElement,
{
    let entity = cx.entity();
    d.tab_index(0).on_key_down(move |ev: &KeyDownEvent, _window, cx| {
        if ev.keystroke.key == "enter" || ev.keystroke.key == "space" {
            let entity = entity.clone();
            entity.update(cx, |this, cx| {
                on_activate(this, cx);
                cx.notify();
            });
        }
    })
}

impl Render for PlinthRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Debug-only timing: set PLINTH_TRACE_RENDER=1 to log each frame's
        // wall time, used to measure the §7.3 list virtualization work.
        #[cfg(debug_assertions)]
        let trace_start = std::env::var_os("PLINTH_TRACE_RENDER").is_some().then(Instant::now);

        // The window's activation is the app's foreground state (`isActive`).
        if self.activation.is_none() {
            self.activation = Some(cx.observe_window_activation(window, |this, window, cx| {
                this.send_lifecycle(window.is_window_active(), cx);
            }));
        }
        self.drive_frames(window, cx);
        self.class = WidthClass::from_width(window.viewport_size().width);
        self.ensure_fields(cx);
        if !self.focused_once {
            self.focused_once = true;
            window.focus(&self.focus, cx);
        }
        let t = Tokens::new(window.appearance(), &self.accent);

        let content = div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .child(self.render_stopped_banner(&t))
            .children(self.render_error_banner(&t, cx))
            .child(match self.tree.current_root() {
                Some(root) => self.render_screen(root, &t, cx),
                None => div().flex_1().into_any_element(),
            });

        // Modal overlays (`Sheet`/`Dialog`, UI API 1.2) and the destructive
        // action confirmation dialog render above everything else.
        let overlay_ids = self.tree.current_root().map(|root| self.overlay_ids(root.id)).unwrap_or_default();
        let mut overlays: Vec<AnyElement> = overlay_ids.into_iter().map(|id| self.render_overlay(id, &t, cx)).collect();
        overlays.extend(self.render_confirm_overlay(&t, cx));
        overlays.extend(self.render_host_dialog_overlay(&t, cx));

        let screens: Vec<(u32, NodeId)> = self.tree.primary_screens().collect();
        let shell = div()
            .id("shell")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .bg(t.background)
            .text_color(t.text)
            // A click anywhere outside a menu's trigger or panel closes it
            // (SPEC.md §6.3); the trigger and the panel itself stop this
            // click from bubbling here.
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                if !this.open_menus.is_empty() {
                    this.open_menus.clear();
                    cx.notify();
                }
            }))
            .on_key_down({
                let entity = cx.entity();
                move |ev: &gpui::KeyDownEvent, _, cx| {
                    let key = ev.keystroke.key.as_str();
                    if key == "escape" {
                        entity.update(cx, |this, cx| {
                            if this.dialog.is_some() {
                                let cancel = match this.dialog.as_ref().map(|d| d.kind) {
                                    Some(DialogKind::Confirm) => Value::Bool(false),
                                    _ => Value::Null,
                                };
                                this.answer_dialog(cancel, cx);
                            } else if !this.open_menus.is_empty() {
                                this.open_menus.clear();
                                cx.notify();
                            } else if this.confirm.is_some() {
                                this.confirm = None;
                                cx.notify();
                            } else {
                                this.tree.go_back();
                                cx.notify();
                            }
                        });
                    } else if key == "left" && ev.keystroke.modifiers.alt {
                        entity.update(cx, |this, cx| {
                            if this.dialog.is_none() && this.open_menus.is_empty() && this.confirm.is_none() {
                                this.tree.go_back();
                                cx.notify();
                            }
                        });
                    } else if key == "enter" {
                        entity.update(cx, |this, cx| {
                            if this.dialog.is_some() {
                                let value = match this.dialog.as_ref() {
                                    Some(d) if d.kind == DialogKind::Confirm => Value::Bool(true),
                                    Some(d) if d.kind == DialogKind::Prompt => {
                                        let text =
                                            d.input.as_ref().map(|s| s.read(cx).as_str().to_owned()).unwrap_or_default();
                                        Value::Str(text)
                                    }
                                    _ => Value::Null,
                                };
                                this.answer_dialog(value, cx);
                            }
                        });
                    }
                }
            });
        let body = if screens.len() < 2 {
            shell.child(content)
        } else {
            match self.class {
                WidthClass::Compact => shell.flex_col().child(content).child(self.render_nav(&screens, &t, cx)),
                WidthClass::Regular | WidthClass::Wide => {
                    shell.flex_row().child(self.render_nav(&screens, &t, cx)).child(content)
                }
            }
        };
        let body = body.children(overlays);

        #[cfg(debug_assertions)]
        if let Some(start) = trace_start {
            eprintln!("plinth: PlinthRoot::render took {:?}", start.elapsed());
        }

        body
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

    /// The last uncaught app error (`error.report`), with a "Dismiss"
    /// button. The host owns this banner, not the app.
    fn render_error_banner(&self, t: &Tokens, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.stopped.is_some() {
            return None;
        }
        let last = self.app_errors.last()?.clone();
        let more = self.app_errors.len() - 1;
        let title: SharedString = if more == 0 { "An error occurred".into() } else { format!("An error occurred ({more} more)").into() };
        let dismiss = div()
            .id("plinth-error-dismiss")
            .role(accesskit::Role::Button)
            .aria_label("Dismiss error")
            .px_3()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(t.on_accent)
            .cursor_pointer()
            .child("Dismiss")
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.app_errors.clear();
                cx.notify();
            }));
        Some(
            div()
                .id("plinth-error-banner")
                .role(accesskit::Role::Alert)
                .aria_label(format!("{title}: {last}"))
                .w_full()
                .px_4()
                .py_3()
                .bg(t.danger)
                .text_color(t.on_accent)
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                        .child(div().text_xs().child(last)),
                )
                .child(dismiss)
                .into_any_element(),
        )
    }

    /// The uncaught app errors that the error banner shows (for tests).
    pub fn app_errors(&self) -> &[String] {
        &self.app_errors
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
        // `<Action>` children are declared through the `actions` prop
        // (SPEC.md §6.3); the compiler appends them as ordinary child
        // nodes, so the renderer tells them apart by control kind.
        let (actions, body): (Vec<NodeId>, Vec<NodeId>) =
            screen.children.iter().partition(|&&c| self.tree.get(c).map(|n| n.kind) == Some(Some(ControlKind::Action)));
        let children: Vec<AnyElement> = body.iter().map(|&c| self.render_node(c, t, cx)).collect();
        let pad = if self.class == WidthClass::Compact { px(16.) } else { px(32.) };
        let can_go_back = self.tree.can_go_back();
        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .id(eid("back", screen.id))
                    .text_lg()
                    .when(can_go_back, |d| {
                        d.cursor_pointer().text_color(t.accent).child("‹").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.tree.go_back();
                            cx.notify();
                        }))
                    }),
            )
            .child(
                div()
                    .id(eid("screen-title", screen.id))
                    .role(accesskit::Role::Heading)
                    .aria_label(title.clone())
                    .flex_1()
                    .text_2xl()
                    .font_weight(FontWeight::BOLD)
                    .child(title),
            )
            .child(self.render_actions_bar(screen.id, &actions, t, cx));
        div()
            .id(eid("screen", screen.id))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .child(div().w_full().max_w(px(720.)).px(pad).py_6().flex().flex_col().gap_5().child(header).children(children))
            .into_any_element()
    }

    /// The screen toolbar (SPEC.md §6.3, §6.4). Overflows into a disclosure
    /// menu past 2 actions on `compact` or 4 on `regular`/`wide`.
    fn render_actions_bar(&self, owner: NodeId, actions: &[NodeId], t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        if actions.is_empty() {
            return div().into_any_element();
        }
        let limit = if self.class == WidthClass::Compact { 2 } else { 4 };
        let (visible, overflow) = if actions.len() <= limit { (actions, &[][..]) } else { actions.split_at(limit - 1) };
        let bar = div()
            .flex()
            .items_center()
            .gap_2()
            .children(visible.iter().filter_map(|&id| self.tree.get(id)).map(|n| self.render_action_item(n, t, false, cx)));
        if overflow.is_empty() {
            return bar.into_any_element();
        }
        let open = self.open_menus.contains(&owner);
        let more = div()
            .id(eid("more", owner))
            .role(accesskit::Role::Button)
            .aria_expanded(open)
            .cursor_pointer()
            .px_2()
            .text_sm()
            .text_color(t.text_muted)
            .child("\u{22EF}")
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.toggle_menu(owner);
                cx.notify();
            }));
        div()
            .flex()
            .flex_col()
            .items_end()
            .gap_1()
            .child(bar.child(more))
            .when(open, |d| {
                d.child(
                    deferred(anchored().snap_to_window().child(self.render_menu_panel(overflow, t, cx))).priority(1),
                )
            })
            .into_any_element()
    }

    /// The floating panel of a `<Menu>` or the screen-toolbar overflow: an
    /// anchored, deferred popover (SPEC.md §6.3) so it floats below its
    /// trigger, above other content, instead of pushing the layout down.
    fn render_menu_panel(&self, actions: &[NodeId], t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("menu-panel")
            .role(accesskit::Role::Menu)
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .mt_1()
            .rounded_lg()
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.stop_propagation()))
            .children(actions.iter().filter_map(|&id| self.tree.get(id)).map(|n| self.render_action_item(n, t, true, cx)))
            .into_any_element()
    }

    /// One `<Action>`: a toolbar button, or a menu row when `in_menu`.
    fn render_action_item(&self, node: &Node, t: &Tokens, in_menu: bool, cx: &mut Context<Self>) -> AnyElement {
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        let glyph = node.str_prop(prop::ICON).map(icon_glyph);
        let destructive = node.enum_prop(prop::ROLE) == button_role::DESTRUCTIVE;
        let id = node.id;
        div()
            .id(eid("action", id))
            .role(if in_menu { accesskit::Role::MenuItem } else { accesskit::Role::Button })
            .aria_label(label.clone())
            .cursor_pointer()
            .px_2()
            .py_1()
            .rounded_md()
            .text_sm()
            .text_color(if destructive { t.danger } else { t.text })
            .hover(|s| s.bg(t.hover))
            .flex()
            .items_center()
            .gap_1()
            .when_some(glyph, |d, g| d.child(g))
            .child(label)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.press_action(id, cx);
            }))
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
            // Sheet and Dialog are modal overlays (SPEC.md §6.3); the host
            // renders them as a layer above the screen, not in the normal
            // flow. See `render()` and `overlay_ids`.
            ControlKind::Sheet | ControlKind::Dialog => div().into_any_element(),
            ControlKind::Tabs => self.render_tabs(node, t, cx),
            ControlKind::Menu => self.render_menu(node, t, cx),
            ControlKind::Grid => self.render_grid(node, t, cx),
            ControlKind::Action => self.render_action_item(node, t, false, cx),
            ControlKind::Image => self.render_image(node, t),
            ControlKind::Icon => self.render_icon(node, t),
            ControlKind::DatePicker => self.render_date_picker(node, t, cx),
            ControlKind::Chart => self.render_chart(node, t),
            ControlKind::Box => self.render_box(node, t, cx),
            ControlKind::Span => self.render_span(node, t),
            ControlKind::Pressable => self.render_pressable(node, t, cx),
            ControlKind::Scroll => self.render_scroll(node, t, cx),
            ControlKind::Canvas => self.render_canvas(node, t, cx),
        }
    }

    /// A Canvas (UI API 1.10): shapes in a view space that scales to the width.
    fn render_canvas(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        warn_if_unlabeled("Canvas", node.id, &label);
        let vw = node.prop(prop::VIEW_WIDTH).and_then(Value::as_int).unwrap_or(100) as f32;
        let vh = node.prop(prop::VIEW_HEIGHT).and_then(Value::as_int).unwrap_or(100) as f32;
        let shapes = crate::canvas::parse(node.str_prop(prop::SHAPES).unwrap_or(""));
        let style = self.primitive_style(node);
        let mut d = crate::canvas::render(u64::from(node.id), label, vw, vh, shapes, t).children(self.pointer_layer(node, Some(vw), cx));
        if let Some(w) = primitives::size(&style, prop::WIDTH, prop::WIDTH_FRACTION) {
            d = d.w(w).flex_shrink_0();
        }
        if let Some(w) = primitives::size(&style, prop::MAX_WIDTH, prop::MAX_WIDTH_FRACTION) {
            d = d.max_w(w);
        }
        primitives::grow(d, &style).into_any_element()
    }

    // -- UI API 1.6: Level 2 styled primitives (docs/UI-ADVANCED.md) --

    /// The children of a Level 2 box. The items of a `List` child (also the
    /// node that `.map()` children make) take part in the layout of the box
    /// itself, so a row of mapped cards is a row.
    fn render_box_children(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let ids = self.box_child_ids(node);
        let style = self.primitive_style(node);
        let range = self.cull_x.get();
        if primitives::is_row(&style) {
            self.row_children(&ids, &style, range, t, cx)
        } else {
            let inner = column_child_range(range, &style);
            ids.iter().map(|&c| self.render_child_in(c, inner, t, cx)).collect()
        }
    }

    /// The children of a box as it lays them out: the items of a `List`
    /// child (`.map()`) are children of the box itself.
    fn box_child_ids(&self, node: &Node) -> Vec<NodeId> {
        let mut ids = Vec::new();
        for &c in &node.children {
            match self.tree.get(c) {
                Some(child) if child.kind == Some(ControlKind::List) => ids.extend(child.children.iter().copied()),
                _ => ids.push(c),
            }
        }
        ids
    }

    /// Builds the child `id` with the visible x range `range` (in the
    /// child's own coordinates). Only boxes and scrolls use the range;
    /// every other control builds its subtree with none.
    fn render_child_in(&self, id: NodeId, range: Option<(f32, f32)>, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let is_box = self.tree.get(id).is_some_and(|n| matches!(n.kind, Some(ControlKind::Box | ControlKind::Scroll)));
        let prev = self.cull_x.replace(if is_box { range } else { None });
        let e = self.render_node(id, t, cx);
        self.cull_x.set(prev);
        e
    }

    /// The children of a row (a row box or a row `Scroll`). With a visible
    /// x range and fixed-width children (justified to the start), it builds
    /// only the children in the range, with a spacer on each side.
    fn row_children(
        &self,
        ids: &[NodeId],
        style: &primitives::Style,
        range: Option<(f32, f32)>,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let pad_left = style.get_f(prop::PADDING_X).or_else(|| style.get_f(prop::PADDING)).map_or(0.0, px_of_units);
        let start = style.enum_(prop::JUSTIFY) == justify::START && !style.flag(prop::WRAP);
        let widths: Option<Vec<f32>> = if start {
            ids.iter().map(|&c| self.tree.get(c).and_then(|n| self.primitive_style(n).get_f(prop::WIDTH)).map(px_of_units)).collect()
        } else {
            None
        };
        let (Some((lo, hi)), Some(widths)) = (range.filter(|_| ids.len() >= CULL_MIN_ROW), widths) else {
            // One child of a start-justified row is at the padding: it can
            // still pass the range on (the content of a horizontal Scroll).
            let one = (start && ids.len() == 1).then(|| range.map(|(lo, hi)| (lo - pad_left, hi - pad_left))).flatten();
            return ids.iter().map(|&c| self.render_child_in(c, one, t, cx)).collect();
        };
        let gap = style.get_f(prop::GAP).map_or(0.0, px_of_units);
        let starts: Vec<f32> = widths
            .iter()
            .scan(pad_left, |x, w| {
                let s = *x;
                *x += w + gap;
                Some(s)
            })
            .collect();
        let shown: Vec<usize> = (0..ids.len()).filter(|&i| starts[i] + widths[i] >= lo && starts[i] <= hi).collect();
        let (Some(&first), Some(&last)) = (shown.first(), shown.last()) else {
            return vec![div().flex_shrink_0().w(px(total_extent(&widths, gap))).into_any_element()];
        };
        let mut out = Vec::with_capacity(last - first + 3);
        if first > 0 {
            out.push(div().flex_shrink_0().w(px((starts[first] - pad_left - gap).max(0.0))).into_any_element());
        }
        for i in first..=last {
            out.push(self.render_child_in(ids[i], Some((lo - starts[i], hi - starts[i])), t, cx));
        }
        if last + 1 < ids.len() {
            let end = starts[last] + widths[last];
            let total = pad_left + total_extent(&widths, gap);
            out.push(div().flex_shrink_0().w(px((total - end - gap).max(0.0))).into_any_element());
        }
        out
    }

    /// The style of a Level 2 element in the current width class, and its
    /// state styles (`hover`, `active`, `focus`, UI API 1.7).
    fn primitive_style(&self, node: &Node) -> primitives::Style {
        primitives::Style::for_class(node, self.class)
    }

    /// Applies the `hover` and `active` partial styles of `node` to `d`.
    fn state_styles<E: StatefulInteractiveElement + Styled>(&self, d: E, node: &Node, t: &Tokens) -> E {
        let mut d = d;
        let tokens = *t;
        if let Some(h) = primitives::partial(node, prop::HOVER) {
            d = d.hover(move |r| primitives::box_style(r, &h, &tokens, true));
        }
        if let Some(a) = primitives::partial(node, prop::ACTIVE) {
            d = d.active(move |r| primitives::box_style(r, &a, &tokens, true));
        }
        d
    }

    /// `onKeyDown`/`onKeyUp` of a Level 2 box (UI API 1.9): the element is a
    /// tab stop and sends the key name; a held key sends no repeats.
    fn key_handlers<E: StatefulInteractiveElement + Styled>(&self, d: E, node: &Node, cx: &mut Context<Self>) -> E {
        let down = node.handler(event::KEY_DOWN).filter(|_| self.stopped.is_none());
        let up = node.handler(event::KEY_UP).filter(|_| self.stopped.is_none());
        if down.is_none() && up.is_none() {
            return d;
        }
        let mut d = d.tab_index(0);
        if let Some(h) = down {
            d = d.on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                if ev.is_held {
                    return;
                }
                if let Some(name) = primitives::key_name(&ev.keystroke.key) {
                    cx.stop_propagation();
                    this.fire(h, event::KEY_DOWN, Value::Str(name), cx);
                }
            }));
        }
        if let Some(h) = up {
            d = d.on_key_up(cx.listener(move |this, ev: &KeyUpEvent, _, cx| {
                if let Some(name) = primitives::key_name(&ev.keystroke.key) {
                    cx.stop_propagation();
                    this.fire(h, event::KEY_UP, Value::Str(name), cx);
                }
            }));
        }
        d
    }

    /// The pointer events of `node` (UI API 1.12) as a full-size layer that
    /// adds mouse listeners when it paints, when it knows the bounds of the
    /// element. `view_width` is the view width of a `Canvas` (the position
    /// is in view units); `None` gives spacing units (a box). `None` if the
    /// node has no pointer handler.
    fn pointer_layer(&self, node: &Node, view_width: Option<f32>, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.stopped.is_some() {
            return None;
        }
        let handlers = [event::POINTER_DOWN, event::POINTER_MOVE, event::POINTER_UP].map(|ev| node.handler(ev));
        if handlers.iter().all(Option::is_none) {
            return None;
        }
        let [down, moved, up] = handlers;
        let entity = cx.entity().downgrade();
        let owner = self.pointer_owner.clone();
        let id = node.id;
        let layer = canvas(
            |_, _, _| (),
            move |bounds: Bounds<Pixels>, _, window, _| {
                let scale = match view_width {
                    Some(vw) => f32::from(bounds.size.width) / vw.max(1.0),
                    None => primitives::UNIT,
                }
                .max(f32::EPSILON);
                let at = move |p: Point<Pixels>| {
                    let x = f32::from(p.x - bounds.origin.x) / scale;
                    let y = f32::from(p.y - bounds.origin.y) / scale;
                    Value::List(vec![Value::Number(f64::from(x)), Value::Number(f64::from(y))])
                };
                let send = {
                    let entity = entity.clone();
                    move |h: u32, ev: u16, value: Value, cx: &mut gpui::App| {
                        let _ = entity.update(cx, |this, cx| this.fire(h, ev, value, cx));
                    }
                };
                {
                    let (owner, send) = (owner.clone(), send.clone());
                    window.on_mouse_event(move |ev: &MouseDownEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble || ev.button != MouseButton::Left || !bounds.contains(&ev.position) {
                            return;
                        }
                        owner.set(Some(id));
                        if let Some(h) = down {
                            send(h, event::POINTER_DOWN, at(ev.position), cx);
                        }
                    });
                }
                {
                    let (owner, send) = (owner.clone(), send.clone());
                    window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                        let Some(h) = moved else { return };
                        if phase == DispatchPhase::Bubble && (owner.get() == Some(id) || bounds.contains(&ev.position)) {
                            send(h, event::POINTER_MOVE, at(ev.position), cx);
                        }
                    });
                }
                window.on_mouse_event(move |ev: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble || ev.button != MouseButton::Left {
                        return;
                    }
                    let owned = owner.get() == Some(id);
                    if owned {
                        owner.set(None);
                    }
                    if let Some(h) = up
                        && (owned || bounds.contains(&ev.position))
                    {
                        send(h, event::POINTER_UP, at(ev.position), cx);
                    }
                });
            },
        );
        Some(layer.absolute().top_0().left_0().size_full().into_any_element())
    }

    /// A layout box. Without a label it is a plain container for AccessKit;
    /// with one it is a named group.
    fn render_box(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let label = node.str_prop(prop::LABEL).map(str::to_owned);
        let style = self.primitive_style(node);
        let d = primitives::box_style(div().id(eid("box", node.id)), &style, t, false);
        let d = self.key_handlers(d, node, cx);
        self.state_styles(d, node, t)
            .when_some(label, |d, l| d.role(accesskit::Role::Group).aria_label(l))
            .children(self.pointer_layer(node, None, cx))
            .children(self.render_box_children(node, t, cx))
            .into_any_element()
    }

    fn render_span(&self, node: &Node, t: &Tokens) -> AnyElement {
        let text = node.text.clone().unwrap_or_default();
        let style = self.primitive_style(node);
        let d = div().id(eid("span", node.id)).role(accesskit::Role::Label).aria_label(text.clone());
        let d = primitives::span_style(d, &style, t);
        let d = match style.enum_(prop::ALIGN) {
            text_align::CENTER => d.text_center(),
            text_align::END => d.text_right(),
            _ => d,
        };
        primitives::grow(d, &style).child(text).into_any_element()
    }

    /// A box that is a button or a link: a tab stop, Enter and Space press
    /// it, and the runtime shows hover and disabled states (or the app's
    /// own `hover`, `active` and `focus` styles).
    fn render_pressable(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        warn_if_unlabeled("Pressable", node.id, &label);
        let disabled = node.bool_prop(prop::DISABLED) || self.stopped.is_some();
        let role = if node.enum_prop(prop::ROLE) == pressable_role::LINK { accesskit::Role::Link } else { accesskit::Role::Button };
        let style = self.primitive_style(node);
        let own_bg = primitives::token(t, style.enum_(prop::BG)).is_some();
        let own_hover = primitives::partial(node, prop::HOVER).is_some();
        let hover = t.hover;
        let d = div().id(eid("pressable", node.id)).role(role).aria_label(label).aria_disabled(disabled);
        let mut d = primitives::box_style(d, &style, t, false)
            .children(self.pointer_layer(node, None, cx))
            .children(self.render_box_children(node, t, cx));
        if disabled {
            d = d.opacity(0.5);
        } else {
            d = d.cursor_pointer();
            d = if own_hover {
                self.state_styles(d, node, t)
            } else {
                let d = d.hover(move |s| if own_bg { s.opacity(0.88) } else { s.bg(hover) });
                match primitives::partial(node, prop::ACTIVE) {
                    Some(a) => {
                        let tokens = *t;
                        d.active(move |r| primitives::box_style(r, &a, &tokens, true))
                    }
                    None => d,
                }
            };
            if let Some(f) = primitives::partial(node, prop::FOCUS) {
                let tokens = *t;
                d = d.focus_visible(move |r| primitives::box_style(r, &f, &tokens, true));
            }
        }
        if !disabled {
            d = self.key_handlers(d, node, cx);
        }
        if let Some(h) = node.handler(event::PRESS).filter(|_| !disabled) {
            d = d
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.fire(h, event::PRESS, Value::Null, cx)))
                .on_a11y_action(accesskit::Action::Click, {
                    let entity = cx.entity();
                    move |_, _, cx| entity.update(cx, |this, cx| this.fire(h, event::PRESS, Value::Null, cx))
                });
            d = keyboard_activatable(d, cx, move |this, cx| this.fire(h, event::PRESS, Value::Null, cx));
        }
        d.into_any_element()
    }

    /// A box that scrolls along its direction.
    fn render_scroll(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let label = node.str_prop(prop::LABEL).map(str::to_owned);
        let style = self.primitive_style(node);
        let handle = self.scroll_handles.borrow_mut().entry(node.id).or_default().clone();
        let d = div().id(eid("scroll", node.id)).role(accesskit::Role::ScrollView).track_scroll(&handle);
        let d = primitives::box_style(d, &style, t, false);
        let d = if primitives::is_row(&style) { d.overflow_x_scroll() } else { d.overflow_y_scroll() };
        let d = self.key_handlers(d, node, cx);
        // A row scroll shows the x range from its offset: its content builds
        // only what is in it (with overscan). A column scroll keeps the range
        // that it got and builds only the rows near its viewport.
        let viewport = handle.bounds().size;
        let outer = self.cull_x.get();
        if primitives::is_row(&style) {
            let w = f32::from(viewport.width);
            let w = if w > 0.0 { w } else { CULL_FIRST_FRAME };
            let lo = -f32::from(handle.offset().x) - CULL_OVERSCAN;
            self.cull_x.set(Some((lo, lo + w + 2.0 * CULL_OVERSCAN)));
        }
        let children = match self.culled_scroll_children(node, &style, &handle, t, cx) {
            Some(children) => children,
            None => self.render_box_children(node, t, cx),
        };
        self.cull_x.set(outer);
        let watch = self.viewport_watch(viewport, cx);
        self.state_styles(d, node, t)
            .when_some(label, |d, l| d.aria_label(l))
            .children(self.pointer_layer(node, None, cx))
            .child(watch)
            .children(children)
            .into_any_element()
    }

    /// An empty layer the size of a `Scroll`: when the scroll's size differs
    /// from `used` (the size that the culling used: unknown before the first
    /// frame, or changed by a resize), it asks for one more frame, so the
    /// built rows and columns follow the new size.
    fn viewport_watch(&self, used: gpui::Size<Pixels>, cx: &mut Context<Self>) -> AnyElement {
        let entity = cx.entity().downgrade();
        canvas(
            move |bounds: Bounds<Pixels>, _, cx| {
                let d = bounds.size - used;
                // The layer is the padding box: a border makes it up to 2 px smaller.
                if f32::from(d.width).abs() > 3.0 || f32::from(d.height).abs() > 3.0 {
                    let entity = entity.clone();
                    cx.defer(move |cx| {
                        let _ = entity.update(cx, |_, cx| cx.notify());
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
    }

    /// The children of a column `Scroll` whose children all have a fixed
    /// height in spacing units (a table, a long list of rows): only the
    /// ones in or near the viewport, with a spacer above and below for the
    /// others, so the scroll range stays the same (docs/GAPS.md 7G-12: 100
    /// rows of 26 cells cost 85 ms a frame when every row was built). The
    /// accessibility tree has only the built rows, as for a virtual `List`.
    /// `None`: build every child (a row scroll, few children, or a child
    /// with no fixed height).
    fn culled_scroll_children(
        &self,
        node: &Node,
        style: &primitives::Style,
        handle: &gpui::ScrollHandle,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> Option<Vec<AnyElement>> {
        if primitives::is_row(style) {
            return None;
        }
        let ids = self.box_child_ids(node);
        if ids.len() < CULL_MIN_CHILDREN {
            return None;
        }
        let px_of = px_of_units;
        let inner = column_child_range(self.cull_x.get(), style);
        let mut heights = Vec::with_capacity(ids.len());
        for &c in &ids {
            let child = self.tree.get(c)?;
            heights.push(px_of(self.primitive_style(child).get_f(prop::HEIGHT)?));
        }
        let gap = style.get_f(prop::GAP).map_or(0.0, px_of);
        let pad_top = style.get_f(prop::PADDING_Y).or_else(|| style.get_f(prop::PADDING)).map_or(0.0, px_of);
        // Before the first frame the viewport is unknown: build a screenful.
        let view_h = f32::from(handle.bounds().size.height);
        let view_h = if view_h > 0.0 { view_h } else { CULL_FIRST_FRAME };
        let view_top = -f32::from(handle.offset().y) - pad_top;
        let (lo, hi) = (view_top - CULL_OVERSCAN, view_top + view_h + CULL_OVERSCAN);
        // The visible range [first, last] and the space before and after it.
        let (mut y, mut first, mut last, mut before) = (0.0f32, None, 0, 0.0f32);
        for (i, h) in heights.iter().enumerate() {
            let end = y + h;
            if end >= lo && y <= hi {
                if first.is_none() {
                    first = Some(i);
                    before = y;
                }
                last = i;
            }
            y = end + gap;
        }
        let total = y - gap;
        let first = first.unwrap_or(ids.len());
        let mut out = Vec::with_capacity(last.saturating_sub(first) + 3);
        // The box's gap also goes after a spacer, so a spacer is one gap shorter.
        if first > 0 {
            out.push(div().flex_shrink_0().h(px((before - gap).max(0.0))).into_any_element());
        }
        if first < ids.len() {
            let after_last: f32 = heights[..=last].iter().sum::<f32>() + gap * last as f32;
            for &c in &ids[first..=last] {
                out.push(self.render_child_in(c, inner, t, cx));
            }
            if last + 1 < ids.len() {
                out.push(div().flex_shrink_0().h(px((total - after_last - gap).max(0.0))).into_any_element());
            }
        }
        Some(out)
    }

    fn render_section(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let title = node.str_prop(prop::TITLE).map(str::to_owned);
        let footer = node.str_prop(prop::FOOTER).map(str::to_owned);
        let heading = title.clone().map(|s| s.to_uppercase());
        div()
            .flex()
            .flex_col()
            .gap_2()
            .when_some(heading, |d, h| d.child(div().px_1().text_xs().text_color(t.text_muted).child(h)))
            .child(
                div()
                    .id(eid("section", node.id))
                    .role(accesskit::Role::Group)
                    .when_some(title.clone(), |d, label| d.aria_label(label))
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
        let d = aligned(div().id(eid("text", node.id)).role(accesskit::Role::Label).aria_label(text.clone()).text_color(color), node);
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
        let d = aligned(
            div().id(eid("heading", node.id)).role(accesskit::Role::Heading).aria_label(text.clone()).font_weight(FontWeight::SEMIBOLD),
            node,
        );
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
        warn_if_unlabeled("Button", node.id, &label);
        let mut d = div()
            .id(eid("btn", node.id))
            .role(accesskit::Role::Button)
            .aria_label(label.clone())
            .aria_disabled(disabled)
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
            .when(!disabled, |d| d.cursor_pointer().hover(move |s| s.bg(hover)));
        if let Some(h) = handler.filter(|_| !disabled) {
            d = d
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.fire(h, event::PRESS, Value::Null, cx)))
                .on_a11y_action(accesskit::Action::Click, {
                    let entity = cx.entity();
                    move |_, _, cx| entity.update(cx, |this, cx| this.fire(h, event::PRESS, Value::Null, cx))
                });
            d = keyboard_activatable(d, cx, move |this, cx| this.fire(h, event::PRESS, Value::Null, cx));
        }
        d.into_any_element()
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
        // An empty `error` means no error (an app writes `cond ? "msg" : ""`).
        let error = node.str_prop(prop::ERROR).filter(|e| !e.is_empty()).map(str::to_owned);
        let Some(field) = self.fields.get(&node.id) else { return div().into_any_element() };
        let id = node.id;
        let disabled = node.bool_prop(prop::DISABLED);
        let input = div()
            .id(eid("tf-wrap", id))
            .w_full()
            .when(disabled, |d| d.opacity(0.5))
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
                    .accepts_input(self.stopped.is_none() && !disabled)
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
        warn_if_unlabeled("Toggle", id, &label);
        let flip = move |this: &mut Self, cx: &mut Context<Self>| {
            let Some(node) = this.tree.get(id) else { return };
            let next = !node.bool_prop(prop::VALUE);
            let handler = node.handler(event::CHANGE);
            this.tree.set_local_prop(id, prop::VALUE, Value::Bool(next));
            match handler {
                Some(h) => this.fire(h, event::CHANGE, Value::Bool(next), cx),
                None => cx.notify(),
            }
        };
        let mut switch = div()
            .id(eid("toggle", id))
            .role(accesskit::Role::Switch)
            .aria_label(label.clone())
            .aria_toggled(if on { accesskit::Toggled::True } else { accesskit::Toggled::False })
            .aria_disabled(disabled)
            .flex_none()
            .w(px(44.))
            .h(px(26.))
            .p(px(3.))
            .rounded_full()
            .flex()
            .when(on, |d| d.justify_end())
            .bg(if on { t.accent } else { t.track })
            .child(div().size(px(20.)).rounded_full().bg(t.knob))
            .when(disabled, |d| d.opacity(0.5));
        if !disabled {
            switch = switch.cursor_pointer().on_click(cx.listener(move |this, _: &ClickEvent, _, cx| flip(this, cx))).on_a11y_action(
                accesskit::Action::Click,
                {
                    let entity = cx.entity();
                    move |_, _, cx| entity.update(cx, |this, cx| flip(this, cx))
                },
            );
            switch = keyboard_activatable(switch, cx, flip);
        }
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
        if count > VIRTUAL_LIST_THRESHOLD {
            return self.render_virtual_list(node, t, cx);
        }
        let rows = node.children.iter().enumerate().map(|(i, &c)| {
            div()
                .when(i + 1 < count, |d| d.border_b_1().border_color(t.border))
                .child(self.render_node(c, t, cx))
        });
        // Rows run edge to edge inside the section card.
        div()
            .id(eid("list", node.id))
            .role(accesskit::Role::List)
            .flex()
            .flex_col()
            .mx(px(-8.))
            .children(rows.collect::<Vec<_>>())
            .into_any_element()
    }

    /// A `List` with more than [`VIRTUAL_LIST_THRESHOLD`] rows renders with
    /// gpui's variable-height `list`/`ListState` (SPEC.md §7.3, Q6:
    /// host-side virtualization): only the rows near the viewport become
    /// elements, so a 10,000-row list costs a near-constant number of
    /// elements per frame instead of one per row. Each row measures its own
    /// height, so mixing rows with and without a subtitle (or any other
    /// content that changes height) no longer clips or gaps rows.
    ///
    /// `PlinthRoot` keeps one [`gpui::ListState`] per virtual `List` node,
    /// keyed by node id, in `self.list_states` (dropped in `apply_commits`
    /// when the node is removed, alongside `self.fields`). When the row
    /// count changes — the simplest correct way to react to a reorder or a
    /// row added/removed — this splices the whole old range for the new
    /// one; `ListState::splice` remeasures the replaced rows but keeps the
    /// scroll position anchored to rows outside the spliced range, so a
    /// prepend or an append does not reset the scroll to the top.
    ///
    /// Because only the rows inside the viewport become elements, rows
    /// scrolled out of view are not present in the AccessKit tree; the list
    /// container keeps `Role::List` so assistive tech still sees it as a
    /// list, but screen readers can only reach the currently visible rows.
    fn render_virtual_list(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let children: Rc<Vec<NodeId>> = Rc::new(node.children.clone());
        let count = children.len();
        let max_h = match self.class {
            WidthClass::Compact => px(420.),
            WidthClass::Regular | WidthClass::Wide => px(560.),
        };
        let state = {
            let mut states = self.list_states.borrow_mut();
            match states.get(&node.id) {
                Some(existing) => {
                    let old_count = existing.item_count();
                    if old_count != count {
                        existing.splice(0..old_count, count);
                    }
                    existing.clone()
                }
                None => {
                    let new_state = gpui::ListState::new(count, gpui::ListAlignment::Top, px(200.));
                    states.insert(node.id, new_state.clone());
                    new_state
                }
            }
        };
        let t = *t;
        let entity = cx.entity();
        let last_id = children.last().copied();
        let list = gpui::list(state, move |i, _window, app| {
            let Some(&c) = children.get(i) else { return div().into_any_element() };
            let last = Some(c) == last_id;
            entity.update(app, |this, cx| {
                let row = this.render_node(c, &t, cx);
                div().when(!last, |d| d.border_b_1().border_color(t.border)).child(row).into_any_element()
            })
        })
        .with_sizing_behavior(gpui::ListSizingBehavior::Auto)
        .h(max_h)
        .into_any_element();
        div()
            .id(eid("list", node.id))
            .role(accesskit::Role::List)
            .flex()
            .flex_col()
            .mx(px(-8.))
            .child(list)
            .into_any_element()
    }

    fn render_row(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let title = node.str_prop(prop::TITLE).unwrap_or("").to_owned();
        let subtitle = node.str_prop(prop::SUBTITLE).map(str::to_owned);
        let glyph = node.str_prop(prop::ICON).map(icon_glyph);
        // An empty `trailing` means none (an app writes `cond ? "text" : ""`).
        let trailing = node.str_prop(prop::TRAILING).filter(|s| !s.is_empty()).map(str::to_owned);
        let handler = node.handler(event::PRESS).filter(|_| self.stopped.is_none());
        let hover = t.hover;
        let aria_label = match &trailing {
            Some(tr) => format!("{title}, {tr}"),
            None => title.clone(),
        };
        let selected = node.bool_prop(prop::SELECTED);
        let row_el = div()
            .id(eid("row", node.id))
            .role(accesskit::Role::ListItem)
            .aria_label(aria_label)
            .aria_selected(selected)
            .when(selected, |d| d.bg(t.selected))
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
            .when_some(trailing, |d, tr| d.child(div().flex_none().text_sm().text_color(t.text_muted).child(tr)))
            .child(div().flex().flex_none().items_center().gap_2().children(self.render_children(node, t, cx)));
        let row_el = match handler {
            Some(h) => keyboard_activatable(
                row_el.cursor_pointer().hover(move |s| s.bg(hover)).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.fire(h, event::PRESS, Value::Null, cx)
                })),
                cx,
                move |this, cx| this.fire(h, event::PRESS, Value::Null, cx),
            ),
            None => row_el,
        };
        row_el.into_any_element()
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
        warn_if_unlabeled("Checkbox", id, &label);
        let flip = move |this: &mut Self, cx: &mut Context<Self>| {
            let Some(node) = this.tree.get(id) else { return };
            let next = !node.bool_prop(prop::VALUE);
            let handler = node.handler(event::CHANGE);
            this.tree.set_local_prop(id, prop::VALUE, Value::Bool(next));
            match handler {
                Some(h) => this.fire(h, event::CHANGE, Value::Bool(next), cx),
                None => cx.notify(),
            }
        };
        let mut box_el = div()
            .id(eid("checkbox", id))
            .role(accesskit::Role::CheckBox)
            .aria_label(label.clone())
            .aria_toggled(if checked { accesskit::Toggled::True } else { accesskit::Toggled::False })
            .aria_disabled(disabled)
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
            .when(disabled, |d| d.opacity(0.5));
        if !disabled {
            box_el = box_el.cursor_pointer().on_click(cx.listener(move |this, _: &ClickEvent, _, cx| flip(this, cx))).on_a11y_action(
                accesskit::Action::Click,
                {
                    let entity = cx.entity();
                    move |_, _, cx| entity.update(cx, |this, cx| flip(this, cx))
                },
            );
            box_el = keyboard_activatable(box_el, cx, flip);
        }
        div().flex().items_center().gap_2().child(box_el).child(div().text_sm().text_color(t.text).child(label)).into_any_element()
    }

    fn render_text_area(&self, node: &Node, t: &Tokens, _cx: &mut Context<Self>) -> AnyElement {
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        let placeholder = node.str_prop(prop::PLACEHOLDER).unwrap_or("").to_owned();
        let Some(field) = self.fields.get(&node.id) else { return div().into_any_element() };
        let id = node.id;
        let disabled = node.bool_prop(prop::DISABLED);
        let input = div().id(eid("ta-wrap", id)).w_full().when(disabled, |d| d.opacity(0.5)).child(
            text_area(eid("ta", id))
                .state(field.state.downgrade())
                .accepts_input(self.stopped.is_none() && !disabled)
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

    /// Sets a `Slider`'s value from a raw pointer position, snapped to
    /// `step` and clamped to `min`/`max` (SPEC.md §8.4). Only sets the
    /// local prop and fires `onChange` when the snapped value actually
    /// changes, so a drag that stays within one step is a no-op.
    fn set_slider_value(&mut self, id: NodeId, raw: f64, cx: &mut Context<Self>) {
        let Some(node) = self.tree.get(id) else { return };
        let min = node.num_prop(prop::MIN).unwrap_or(0.0);
        let max = node.num_prop(prop::MAX).unwrap_or(1.0);
        let step = node.num_prop(prop::STEP).unwrap_or(((max - min) / 20.0).max(0.000_1));
        let current = node.num_prop(prop::VALUE).unwrap_or(min).clamp(min, max);
        let next = snap_slider_value(raw, min, max, step);
        if next == current {
            return;
        }
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
        warn_if_unlabeled("Slider", id, &label);
        let frac = if max > min { ((value - min) / (max - min)).clamp(0.0, 1.0) } else { 0.0 };
        let track = div()
            .id(eid("slider-track", id))
            .role(accesskit::Role::Slider)
            .aria_label(label.clone())
            .aria_numeric_value(value)
            .aria_numeric_value_step(step)
            .aria_disabled(disabled)
            .relative()
            .flex_1()
            .h(px(8.))
            .rounded_full()
            .bg(t.track)
            .child(div().h_full().rounded_full().bg(t.accent).w(gpui::relative(frac as f32)));
        let track_bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
        let track = if disabled {
            div().w_full().child(track)
        } else {
            let inc = move |this: &mut Self, cx: &mut Context<Self>| this.step_value(id, step, cx);
            let dec = move |this: &mut Self, cx: &mut Context<Self>| this.step_value(id, -step, cx);
            let track = track
                .on_a11y_action(accesskit::Action::Increment, {
                    let entity = cx.entity();
                    move |_, _, cx| entity.update(cx, |this, cx| inc(this, cx))
                })
                .on_a11y_action(accesskit::Action::Decrement, {
                    let entity = cx.entity();
                    move |_, _, cx| entity.update(cx, |this, cx| dec(this, cx))
                });
            let entity = cx.entity();
            let track = track.tab_index(0).on_key_down(move |ev: &KeyDownEvent, _window, cx| {
                let delta = match ev.keystroke.key.as_str() {
                    "right" | "up" => Some(step),
                    "left" | "down" => Some(-step),
                    _ => None,
                };
                if let Some(d) = delta {
                    let entity = entity.clone();
                    entity.update(cx, |this, cx| this.step_value(id, d, cx));
                }
            });
            // Pointer drag on the track (SPEC.md §8.4): a click or drag at
            // position `x` maps to a value by `x`'s fraction across the
            // track's painted bounds, which `on_children_prepainted` caches.
            let value_from_x = move |x: Pixels, bounds: Bounds<Pixels>| -> f64 {
                let offset = (x - bounds.left()).clamp(px(0.), bounds.size.width);
                let frac = if bounds.size.width > px(0.) { (offset / bounds.size.width) as f64 } else { 0.0 };
                min + (max - min) * frac
            };
            let down_bounds = track_bounds.clone();
            let down_entity = cx.entity();
            let track = track.on_mouse_down(MouseButton::Left, move |ev, _window, app| {
                if let Some(bounds) = down_bounds.get() {
                    let raw = value_from_x(ev.position.x, bounds);
                    let entity = down_entity.clone();
                    entity.update(app, |this, cx| this.set_slider_value(id, raw, cx));
                }
            });
            let drag_entity = cx.entity();
            let track = track
                .on_drag(SliderDrag { id }, |drag, _, _, cx| cx.new(|_| drag.clone()))
                .on_drag_move(move |ev: &DragMoveEvent<SliderDrag>, _window, app| {
                    if ev.drag(app).id != id {
                        return;
                    }
                    let raw = value_from_x(ev.event.position.x, ev.bounds);
                    let entity = drag_entity.clone();
                    entity.update(app, |this, cx| this.set_slider_value(id, raw, cx));
                });
            div().w_full().on_children_prepainted(move |bounds, _window, _cx| track_bounds.set(bounds.first().copied())).child(track)
        };
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
        warn_if_unlabeled("NumberField", id, &label);
        let input = div()
            .id(eid("nf-wrap", id))
            .role(accesskit::Role::SpinButton)
            .aria_label(label.clone())
            .aria_numeric_value(node.num_prop(prop::VALUE).unwrap_or(0.0))
            .aria_numeric_value_step(step)
            .aria_disabled(disabled)
            .flex()
            .items_center()
            .gap_1()
            .child(self.step_button(id, "-", -step, disabled, t, cx))
            .child(
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
        warn_if_unlabeled("Picker", id, &label);
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
                    .role(accesskit::Role::RadioButton)
                    .aria_label(opt.clone())
                    .aria_selected(selected)
                    .aria_disabled(disabled)
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
        let mut bar = div().id(eid("progress", node.id)).role(accesskit::Role::ProgressIndicator);
        if let Some(l) = &label {
            bar = bar.aria_label(l.clone());
        }
        if let Some(v) = value {
            bar = bar.aria_numeric_value(v as f64);
        }
        let bar = bar.w_full().h(px(6.)).rounded_full().bg(t.track).child(match value {
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

    /// `<Image>` (SPEC.md §6.3, §10.1): an asset from the package. The
    /// runtime sizes it by `aspect`, not pixels. A missing or unreadable
    /// asset falls back to a placeholder box with the `alt` text, so a
    /// broken image never breaks accessibility.
    fn render_image(&self, node: &Node, t: &Tokens) -> AnyElement {
        let src = node.str_prop(prop::SRC).unwrap_or("");
        let alt = node.str_prop(prop::ALT).unwrap_or("").to_owned();
        let aspect_kind = node.enum_prop(prop::ASPECT);
        let ratio = match aspect_kind {
            aspect::WIDE => 16.0 / 9.0,
            aspect::TALL => 3.0 / 4.0,
            _ => 1.0,
        };
        // Regular/wide windows cap the image height and center it
        // horizontally, instead of filling the full content width
        // (SPEC.md §6.3). Compact keeps the full-width default.
        let max_h = match self.class {
            WidthClass::Compact => None,
            WidthClass::Regular | WidthClass::Wide => Some(match aspect_kind {
                aspect::WIDE => 280.,
                _ => 360.,
            }),
        };
        let id = eid("image", node.id);
        match self.assets.get(src).and_then(|bytes| image_format(src).map(|f| (f, bytes))) {
            Some((format, bytes)) => div()
                .id(id)
                .role(accesskit::Role::Image)
                .aria_label(alt)
                .when(max_h.is_none(), |d| d.w_full())
                .when_some(max_h, |d, h| d.h(px(h)).max_w_full().mx_auto())
                .aspect_ratio(ratio as f32)
                .overflow_hidden()
                .rounded_md()
                .child(
                    img(Arc::new(Image::from_bytes(format, bytes.clone())))
                        .w_full()
                        .h_full()
                        .object_fit(ObjectFit::Cover),
                )
                .into_any_element(),
            None => div()
                .id(id)
                .role(accesskit::Role::Image)
                .aria_label(alt.clone())
                .when(max_h.is_none(), |d| d.w_full())
                .when_some(max_h, |d, h| d.h(px(h)).max_w_full().mx_auto())
                .aspect_ratio(ratio as f32)
                .rounded_md()
                .bg(t.surface_alt)
                .border_1()
                .border_color(t.border)
                .flex()
                .items_center()
                .justify_center()
                .p_2()
                .child(div().text_xs().text_color(t.text_muted).child(alt))
                .into_any_element(),
        }
    }

    /// `<Icon>` (SPEC.md §6.3, UI API 1.4): a glyph from the runtime icon
    /// set. Decorative by default (hidden from AccessKit); `label` gives
    /// it an accessible name.
    fn render_icon(&self, node: &Node, t: &Tokens) -> AnyElement {
        let glyph = icon_glyph(node.str_prop(prop::ICON).unwrap_or(""));
        let color = match node.enum_prop(prop::TONE) {
            tone::MUTED => t.text_muted,
            tone::DANGER => t.danger,
            tone::SUCCESS => t.success,
            _ => t.text,
        };
        let label = node.str_prop(prop::LABEL).map(str::to_owned);
        let el = div().id(eid("icon", node.id)).text_color(color).child(glyph);
        match label {
            Some(label) => el.role(accesskit::Role::Image).aria_label(label),
            None => el.aria_hidden(),
        }
        .into_any_element()
    }

    /// `<Chart>` (SPEC.md §6.3, UI API 1.5): a data-driven bar, line or pie
    /// chart. The runtime picks the colors (the theme's chart palette),
    /// height and labels; the app only supplies `label`, `kind` and data.
    /// Empty data shows a "No data" placeholder. The role is `Figure`,
    /// named by `label`, with a hidden text summary of every value so a
    /// screen reader gets the numbers (there is no visual data table).
    fn render_chart(&self, node: &Node, t: &Tokens) -> AnyElement {
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        warn_if_unlabeled("Chart", node.id, &label);
        let kind = node.enum_prop(prop::CHART_KIND);
        let series = chart_series(node);
        let id = eid("chart", node.id);

        if series.is_empty() || series.iter().all(|(_, pts)| pts.is_empty()) {
            return div()
                .id(id)
                .role(accesskit::Role::Figure)
                .aria_label(label)
                .flex()
                .items_center()
                .justify_center()
                .h(px(120.))
                .rounded_lg()
                .bg(t.surface_alt)
                .text_color(t.text_muted)
                .text_sm()
                .child("No data")
                .into_any_element();
        }

        let desc = chart_description(&series);
        let chart_h = match self.class { WidthClass::Compact => 160., _ => 220. };
        let compact = self.class == WidthClass::Compact;
        let body = match kind {
            // The pie draws its own legend (with each share) beside it,
            // or below it at compact width.
            chart_kind::PIE => crate::chart::render_pie(&series[0].1, t, if compact { 150. } else { 180. }, compact),
            chart_kind::LINE => {
                let max_x_labels = match self.class { WidthClass::Compact => 6, WidthClass::Regular => 10, _ => 14 };
                crate::chart::render_line(&series, t, chart_h, max_x_labels)
            }
            _ => crate::chart::render_bars(&series, t, chart_h),
        };
        let legend = (kind != chart_kind::PIE && series.len() > 1).then(|| {
            let names: Vec<String> = series.iter().map(|(n, _)| n.clone()).collect();
            render_legend(&names, t)
        });

        div()
            .id(id)
            .role(accesskit::Role::Figure)
            .aria_label(label)
            .flex()
            .flex_col()
            .gap_2()
            .child(body)
            .children(legend)
            // A visually hidden description with every value, so
            // AccessKit (and `plinth-shoot`'s a11y check) exposes the
            // numbers without relying on the drawing.
            .child(
                div()
                    .id(eid("chart-data", node.id))
                    .role(accesskit::Role::Label)
                    .aria_label(desc)
                    .invisible()
                    .absolute()
                    .w(px(0.))
                    .h(px(0.))
                    .overflow_hidden(),
            )
            .into_any_element()
    }

    /// `<DatePicker>` (SPEC.md §6.3, UI API 1.4): a labelled field that
    /// shows the value in a readable English form and opens an anchored,
    /// deferred popover (the `Menu` pattern) with a month grid, time
    /// steppers, or both, depending on `mode`.
    fn render_date_picker(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let id = node.id;
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        warn_if_unlabeled("DatePicker", id, &label);
        let mode = match node.enum_prop(prop::MODE) {
            date_picker_mode::TIME => "time",
            date_picker_mode::DATETIME => "datetime",
            _ => "date",
        };
        let value = node.str_prop(prop::VALUE).unwrap_or("").to_owned();
        let (date, time) = calendar::parse_value(mode, &value);
        let display = match (date, time) {
            (Some((y, m, d)), Some((h, mi))) => format!("{} {}", calendar::format_date_readable(y, m, d), calendar::format_time_readable(h, mi)),
            (Some((y, m, d)), None) => calendar::format_date_readable(y, m, d),
            (None, Some((h, mi))) => calendar::format_time_readable(h, mi),
            (None, None) => "No date".to_owned(),
        };
        let disabled = node.bool_prop(prop::DISABLED) || self.stopped.is_some();
        let open = !disabled && self.open_menus.contains(&id);
        let trigger = div()
            .id(eid("dp-trigger", id))
            .role(accesskit::Role::Button)
            .aria_label(format!("{label}: {display}"))
            .aria_expanded(open)
            .aria_disabled(disabled)
            .when(disabled, |d| d.opacity(0.5))
            .when(!disabled, |d| d.cursor_pointer())
            .w_full()
            .px_3()
            .py_2()
            .rounded_lg()
            .border_1()
            .border_color(t.border)
            .bg(t.background)
            .text_color(t.text)
            .text_sm()
            .flex()
            .items_center()
            .justify_between()
            .child(display)
            .child(div().text_color(t.text_muted).child(icon_glyph(if mode == "time" { "clock" } else { "calendar" })));
        let trigger = if disabled {
            trigger
        } else {
            let trigger = trigger.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.toggle_date_picker(id);
                cx.notify();
            }));
            keyboard_activatable(trigger, cx, move |this, cx| {
                this.toggle_date_picker(id);
                cx.notify();
            })
        };
        let mut wrap = div().flex().flex_col().gap_1().child(trigger);
        if open {
            wrap = wrap.child(
                deferred(
                    anchored()
                        .snap_to_window()
                        .child(
                            div()
                                .id(eid("dp-popover", id))
                                .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.stop_propagation()))
                                .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                                    this.date_picker_key(id, mode, &ev.keystroke.key, cx);
                                }))
                                .child(self.render_date_picker_panel(id, mode, date, time, t, cx)),
                        ),
                )
                .priority(1),
            );
        }
        self.labelled(label, wrap, None, t)
    }

    fn toggle_date_picker(&mut self, id: NodeId) {
        if self.open_menus.remove(&id) {
            self.date_cursor.remove(&id);
            return;
        }
        self.open_menus.clear();
        self.date_cursor.clear();
        self.open_menus.insert(id);
        let mode = self.tree.get(id).map(|n| match n.enum_prop(prop::MODE) {
            date_picker_mode::TIME => "time",
            date_picker_mode::DATETIME => "datetime",
            _ => "date",
        }).unwrap_or("date");
        let value = self.tree.get(id).and_then(|n| n.str_prop(prop::VALUE)).unwrap_or("").to_owned();
        let (date, _) = calendar::parse_value(mode, &value);
        let (y, m, d) = date.unwrap_or_else(calendar::today);
        self.date_cursor.insert(id, (y, m, d));
    }

    /// Commits a new value for a `DatePicker` and fires `change`
    /// (SPEC.md §8.4: the host sets its value first, then sends `change`).
    fn commit_date_picker(&mut self, id: NodeId, mode: &str, date: Option<(i32, u32, u32)>, time: Option<(u32, u32)>, cx: &mut Context<Self>) {
        let text = calendar::format_value(mode, date, time);
        let handler = self.tree.get(id).and_then(|n| n.handler(event::CHANGE));
        self.tree.set_local_prop(id, prop::VALUE, Value::Str(text.clone()));
        match handler {
            Some(h) => self.fire(h, event::CHANGE, Value::Str(text), cx),
            None => cx.notify(),
        }
    }

    /// Keyboard handling inside an open `DatePicker` popover (SPEC.md §6.3):
    /// arrows move the focused day, PageUp/PageDown change month, Enter
    /// picks, Escape closes.
    fn date_picker_key(&mut self, id: NodeId, mode: &str, key: &str, cx: &mut Context<Self>) {
        if mode == "time" {
            if key == "escape" {
                self.open_menus.remove(&id);
                self.date_cursor.remove(&id);
                cx.notify();
            }
            return;
        }
        let Some(&(mut y, mut m, mut focus_day)) = self.date_cursor.get(&id) else { return };
        match key {
            "left" => focus_day = focus_day.saturating_sub(1).max(1),
            "right" => focus_day += 1,
            "up" => focus_day = focus_day.saturating_sub(7).max(1),
            "down" => focus_day += 7,
            "pageup" => {
                let (ny, nm) = calendar::add_months(y, m, -1);
                y = ny;
                m = nm;
                focus_day = calendar::clamp_day(y, m, focus_day);
            }
            "pagedown" => {
                let (ny, nm) = calendar::add_months(y, m, 1);
                y = ny;
                m = nm;
                focus_day = calendar::clamp_day(y, m, focus_day);
            }
            "enter" => {
                let value = self.tree.get(id).and_then(|n| n.str_prop(prop::VALUE)).unwrap_or("").to_owned();
                let (_, time) = calendar::parse_value(mode, &value);
                self.commit_date_picker(id, mode, Some((y, m, focus_day)), time, cx);
                self.open_menus.remove(&id);
                self.date_cursor.remove(&id);
                cx.notify();
                return;
            }
            "escape" => {
                self.open_menus.remove(&id);
                self.date_cursor.remove(&id);
                cx.notify();
                return;
            }
            _ => return,
        }
        // Rolling past the end/start of the visible month moves to the
        // next/previous one, keeping the focused day in range.
        let days = calendar::days_in_month(y, m);
        if focus_day > days {
            let (ny, nm) = calendar::add_months(y, m, 1);
            y = ny;
            m = nm;
            focus_day = 1;
        }
        self.date_cursor.insert(id, (y, m, focus_day));
        cx.notify();
    }

    /// The popover body: a month grid for `date`/`datetime`, hour/minute
    /// steppers for `time`/`datetime`.
    fn render_date_picker_panel(
        &self,
        id: NodeId,
        mode: &str,
        date: Option<(i32, u32, u32)>,
        time: Option<(u32, u32)>,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut panel = div().id(eid("dp-panel", id)).flex().flex_col().gap_3().p_3().mt_1().rounded_lg().bg(t.surface).border_1().border_color(t.border);
        if mode != "time" {
            panel = panel.child(self.render_date_grid(id, date, t, cx));
        }
        if mode != "date" {
            panel = panel.child(self.render_time_steppers(id, mode, date, time, t, cx));
        }
        panel.into_any_element()
    }

    fn render_date_grid(&self, id: NodeId, date: Option<(i32, u32, u32)>, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let (year, month, focus_day) = self.date_cursor.get(&id).copied().unwrap_or_else(|| {
            let (y, m, d) = date.unwrap_or_else(calendar::today);
            (y, m, d)
        });
        let (today_y, today_m, today_d) = calendar::today();
        let selected_day = date.filter(|&(y, m, _)| y == year && m == month).map(|(_, _, d)| d);
        let prev = move |this: &mut Self, cx: &mut Context<Self>| {
            if let Some(&(y, m, d)) = this.date_cursor.get(&id) {
                let (ny, nm) = calendar::add_months(y, m, -1);
                this.date_cursor.insert(id, (ny, nm, calendar::clamp_day(ny, nm, d)));
                cx.notify();
            }
        };
        let next = move |this: &mut Self, cx: &mut Context<Self>| {
            if let Some(&(y, m, d)) = this.date_cursor.get(&id) {
                let (ny, nm) = calendar::add_months(y, m, 1);
                this.date_cursor.insert(id, (ny, nm, calendar::clamp_day(ny, nm, d)));
                cx.notify();
            }
        };
        let nav = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .id(eid("dp-prev", id))
                    .role(accesskit::Role::Button)
                    .aria_label("Previous month")
                    .cursor_pointer()
                    .px_2()
                    .text_color(t.text)
                    .child("\u{2039}")
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| prev(this, cx))),
            )
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(format!("{} {year}", calendar::month_name(month))))
            .child(
                div()
                    .id(eid("dp-next", id))
                    .role(accesskit::Role::Button)
                    .aria_label("Next month")
                    .cursor_pointer()
                    .px_2()
                    .text_color(t.text)
                    .child("\u{203A}")
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| next(this, cx))),
            );
        let weekday_header = div()
            .flex()
            .children(["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"].iter().map(|d| {
                div().flex_1().flex().items_center().justify_center().text_xs().text_color(t.text_muted).child(*d)
            }));
        let first_weekday = calendar::weekday_of(year, month, 1);
        let days_in_month = calendar::days_in_month(year, month);
        let mut cells: Vec<AnyElement> = Vec::new();
        for _ in 0..first_weekday {
            cells.push(div().flex_1().into_any_element());
        }
        for day in 1..=days_in_month {
            let is_today = (year, month, day) == (today_y, today_m, today_d);
            let is_selected = selected_day == Some(day);
            let is_focused = day == focus_day;
            let hover = t.hover;
            let mut cell = div()
                .id(eid("dp-day", (id as u64) * 1000 + day as u64))
                .role(accesskit::Role::GridCell)
                .aria_selected(is_selected)
                .aria_label(calendar::format_date_readable(year, month, day))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .py_1()
                .mx_px()
                .rounded_md()
                .cursor_pointer()
                .text_sm()
                .hover(|s| s.bg(hover))
                .when(is_selected, |d| d.bg(t.accent).text_color(t.on_accent))
                .when(!is_selected, |d| d.text_color(t.text))
                .when(is_today && !is_selected, |d| d.border_1().border_color(t.accent))
                .when(is_focused, |d| d.border_1().border_color(t.text_muted))
                .child(format!("{day}"));
            cell = cell.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                let mode = this.tree.get(id).map(|n| match n.enum_prop(prop::MODE) {
                    date_picker_mode::TIME => "time",
                    date_picker_mode::DATETIME => "datetime",
                    _ => "date",
                }).unwrap_or("date");
                let value = this.tree.get(id).and_then(|n| n.str_prop(prop::VALUE)).unwrap_or("").to_owned();
                let (_, time) = calendar::parse_value(mode, &value);
                this.commit_date_picker(id, mode, Some((year, month, day)), time, cx);
                let close_now = mode != "datetime";
                if close_now {
                    this.open_menus.remove(&id);
                    this.date_cursor.remove(&id);
                }
                cx.notify();
            }));
            cells.push(cell.into_any_element());
        }
        let total = first_weekday + days_in_month;
        let trailing = (7 - total % 7) % 7;
        for _ in 0..trailing {
            cells.push(div().flex_1().into_any_element());
        }
        let mut grid_rows: Vec<AnyElement> = Vec::new();
        let mut it = cells.into_iter();
        loop {
            let row: Vec<AnyElement> = it.by_ref().take(7).collect();
            if row.is_empty() {
                break;
            }
            grid_rows.push(div().flex().children(row).into_any_element());
        }
        div()
            .id(eid("dp-grid", id))
            .role(accesskit::Role::Grid)
            .flex()
            .flex_col()
            .gap_1()
            .w(px(240.))
            .child(nav)
            .child(weekday_header)
            .children(grid_rows)
            .into_any_element()
    }

    fn render_time_steppers(
        &self,
        id: NodeId,
        mode: &str,
        date: Option<(i32, u32, u32)>,
        time: Option<(u32, u32)>,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (hour, minute) = time.unwrap_or((0, 0));
        let mode_owned = mode.to_owned();
        let step = move |this: &mut Self, dh: i32, dm: i32, cx: &mut Context<Self>| {
            let value = this.tree.get(id).and_then(|n| n.str_prop(prop::VALUE)).unwrap_or("").to_owned();
            let (d, t) = calendar::parse_value(&mode_owned, &value);
            let (h, m) = t.unwrap_or((0, 0));
            let nh = ((h as i32 + dh).rem_euclid(24)) as u32;
            let nm = ((m as i32 + dm).rem_euclid(60)) as u32;
            this.commit_date_picker(id, &mode_owned, d, Some((nh, nm)), cx);
            cx.notify();
        };
        let step_h_up = step.clone();
        let hour_up = move |this: &mut Self, cx: &mut Context<Self>| step_h_up(this, 1, 0, cx);
        let step_h_down = step.clone();
        let hour_down = move |this: &mut Self, cx: &mut Context<Self>| step_h_down(this, -1, 0, cx);
        let step_m_up = step.clone();
        let min_up = move |this: &mut Self, cx: &mut Context<Self>| step_m_up(this, 0, 1, cx);
        let step_m_down = step.clone();
        let min_down = move |this: &mut Self, cx: &mut Context<Self>| step_m_down(this, 0, -1, cx);
        let stepper = |value: String, up: Box<dyn Fn(&mut Self, &mut Context<Self>)>, down: Box<dyn Fn(&mut Self, &mut Context<Self>)>, label: &'static str, cx: &mut Context<Self>| {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .id(SharedString::from(format!("{label}-up-{id}")))
                        .role(accesskit::Role::Button)
                        .aria_label(format!("Increase {label}"))
                        .cursor_pointer()
                        .px_2()
                        .text_color(t.text)
                        .child("+")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| up(this, cx))),
                )
                .child(div().w(px(36.)).text_center().text_sm().child(value))
                .child(
                    div()
                        .id(SharedString::from(format!("{label}-down-{id}")))
                        .role(accesskit::Role::Button)
                        .aria_label(format!("Decrease {label}"))
                        .cursor_pointer()
                        .px_2()
                        .text_color(t.text)
                        .child("\u{2212}")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| down(this, cx))),
                )
                .into_any_element()
        };
        let _ = date;
        div()
            .flex()
            .items_center()
            .justify_center()
            .gap_3()
            .child(stepper(format!("{hour:02}"), Box::new(hour_up), Box::new(hour_down), "hour", cx))
            .child(div().text_sm().child(":"))
            .child(stepper(format!("{minute:02}"), Box::new(min_up), Box::new(min_down), "minute", cx))
            .into_any_element()
    }

    // -- UI API 1.2 structure --

    fn toggle_menu(&mut self, id: NodeId) {
        if !self.open_menus.remove(&id) {
            self.open_menus.clear();
            self.open_menus.insert(id);
        }
    }

    /// Presses an `<Action>`. A destructive action asks for confirmation
    /// first, unless `confirm={false}` (SPEC.md §6.4).
    fn press_action(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(node) = self.tree.get(id) else { return };
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        let handler = node.handler(event::PRESS);
        let destructive = node.enum_prop(prop::ROLE) == button_role::DESTRUCTIVE;
        let needs_confirm = destructive && node.prop(prop::CONFIRM).and_then(Value::as_bool).unwrap_or(true);
        self.open_menus.clear();
        if needs_confirm {
            self.confirm = Some(PendingConfirm { label, handler });
        } else if let Some(h) = handler {
            self.fire(h, event::PRESS, Value::Null, cx);
        }
        cx.notify();
    }

    /// The standard confirmation dialog for a destructive action (SPEC.md
    /// §6.4). This dialog is host state, not a guest node.
    fn render_confirm_overlay(&self, t: &Tokens, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pending = self.confirm.as_ref()?;
        let message = format!("Are you sure you want to {}? This cannot be undone.", pending.label.to_lowercase());
        let handler = pending.handler;
        let panel = div()
            .id("confirm-panel")
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.stop_propagation()))
            .w(px(320.))
            .p_5()
            .flex()
            .flex_col()
            .gap_4()
            .rounded_xl()
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(pending.label.clone()))
            .child(div().text_sm().text_color(t.text_muted).child(message))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        div()
                            .id("confirm-cancel")
                            .cursor_pointer()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .text_sm()
                            .child("Cancel")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.confirm = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("confirm-ok")
                            .cursor_pointer()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(t.danger)
                            .text_color(t.on_accent)
                            .text_sm()
                            .child("Delete")
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.confirm = None;
                                if let Some(h) = handler {
                                    this.fire(h, event::PRESS, Value::Null, cx);
                                }
                                cx.notify();
                            })),
                    ),
            );
        Some(
            div()
                .id("confirm-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(t.backdrop)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.confirm = None;
                    cx.notify();
                }))
                .child(panel)
                .into_any_element(),
        )
    }

    /// Node ids of the open `Sheet`/`Dialog` descendants of `root`
    /// (SPEC.md §6.3). There is normally at most one at a time.
    fn overlay_ids(&self, root: NodeId) -> Vec<NodeId> {
        let mut found = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(node) = self.tree.get(id) else { continue };
            let is_overlay = matches!(node.kind, Some(ControlKind::Sheet) | Some(ControlKind::Dialog));
            if is_overlay && node.bool_prop(prop::VALUE) {
                found.push(id);
            }
            stack.extend(node.children.iter().copied());
        }
        found
    }

    /// Closes `id` (a `Sheet` or `Dialog`), updating a bound `open` signal
    /// two ways (SPEC.md §8.4).
    fn close_overlay(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let handler_close = self.tree.get(id).and_then(|n| n.handler(event::CLOSE));
        let handler_change = self.tree.get(id).and_then(|n| n.handler(event::CHANGE));
        self.tree.set_local_prop(id, prop::VALUE, Value::Bool(false));
        if let Some(h) = handler_change {
            self.fire(h, event::CHANGE, Value::Bool(false), cx);
        }
        if let Some(h) = handler_close {
            self.fire(h, event::PRESS, Value::Null, cx);
        }
        cx.notify();
    }

    fn render_overlay(&self, id: NodeId, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let Some(node) = self.tree.get(id) else { return div().into_any_element() };
        match node.kind {
            Some(ControlKind::Dialog) => self.render_dialog_overlay(node, t, cx),
            _ => self.render_sheet_overlay(node, t, cx),
        }
    }

    fn render_dialog_overlay(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let id = node.id;
        let title = node.str_prop(prop::TITLE).unwrap_or("").to_owned();
        let message = node.str_prop(prop::MESSAGE).map(str::to_owned);
        let actions: Vec<&Node> = node.children.iter().filter_map(|&c| self.tree.get(c)).collect();
        let panel = div()
            .id(eid("dialog", id))
            .role(accesskit::Role::Dialog)
            .aria_modal(true)
            .aria_label(title.clone())
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.stop_propagation()))
            .w(px(360.))
            .p_5()
            .flex()
            .flex_col()
            .gap_4()
            .rounded_xl()
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(title))
            .when_some(message, |d, m| d.child(div().text_sm().text_color(t.text_muted).child(m)))
            .child(div().flex().justify_end().gap_2().children(actions.into_iter().map(|a| self.render_action_item(a, t, false, cx))));
        div()
            .id(eid("dialog-backdrop", id))
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.backdrop)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.close_overlay(id, cx)))
            .child(panel)
            .into_any_element()
    }

    /// A bottom sheet on `compact`, a side panel on `regular`/`wide`
    /// (SPEC.md §6.3).
    fn render_sheet_overlay(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let id = node.id;
        let title = node.str_prop(prop::TITLE).unwrap_or("").to_owned();
        let children: Vec<AnyElement> = node.children.iter().map(|&c| self.render_node(c, t, cx)).collect();
        let close_btn = div()
            .id(eid("sheet-close", id))
            .cursor_pointer()
            .text_color(t.text_muted)
            .child("\u{2715}")
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.close_overlay(id, cx)));
        let panel = div()
            .id(eid("sheet", id))
            .role(accesskit::Role::Dialog)
            .aria_modal(true)
            .aria_label(title.clone())
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.stop_propagation()))
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .rounded_t_xl()
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .child(div().flex().items_center().justify_between().child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(title)).child(close_btn))
            .children(children);
        let backdrop = div()
            .id(eid("sheet-backdrop", id))
            .absolute()
            .inset_0()
            .bg(t.backdrop)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.close_overlay(id, cx)));
        if self.class == WidthClass::Compact {
            backdrop.flex().flex_col().justify_end().child(panel.w_full().max_h(px(480.)).overflow_y_scroll()).into_any_element()
        } else {
            backdrop.flex().justify_end().child(panel.h_full().w(px(360.)).rounded_l_xl().overflow_y_scroll()).into_any_element()
        }
    }

    /// In-screen segmented tabs (SPEC.md §6.3). `items` is joined with
    /// U+001F (see `controls.rs::PropTy::StrList`).
    fn render_tabs(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let items: Vec<&str> = node.str_prop(prop::ITEMS).map(|s| s.split('\u{1f}').collect()).unwrap_or_default();
        let current = node.str_prop(prop::VALUE).unwrap_or("").to_owned();
        let id = node.id;
        div()
            .id(eid("tablist", id))
            .role(accesskit::Role::TabList)
            .flex()
            .p_1()
            .gap_1()
            .rounded_lg()
            .bg(t.surface_alt)
            .children(items.into_iter().enumerate().map(|(i, item)| {
                let item = item.to_owned();
                let selected = item == current;
                let value = item.clone();
                div()
                    .id(eid("tab", id as u64 * 1000 + i as u64))
                    .role(accesskit::Role::Tab)
                    .aria_label(item.clone())
                    .aria_selected(selected)
                    .flex_1()
                    .cursor_pointer()
                    .text_center()
                    .text_sm()
                    .py_1()
                    .rounded_md()
                    .when(selected, |d| d.bg(t.surface).font_weight(FontWeight::SEMIBOLD))
                    .child(item)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let handler = this.tree.get(id).and_then(|n| n.handler(event::CHANGE));
                        this.tree.set_local_prop(id, prop::VALUE, Value::Str(value.clone()));
                        match handler {
                            Some(h) => this.fire(h, event::CHANGE, Value::Str(value.clone()), cx),
                            None => cx.notify(),
                        }
                    }))
            }))
            .into_any_element()
    }

    /// `<Menu>`: a label that discloses its actions (SPEC.md §6.3).
    fn render_menu(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let label = node.str_prop(prop::LABEL).unwrap_or("").to_owned();
        let id = node.id;
        let open = self.open_menus.contains(&id);
        let actions: Vec<NodeId> = node.children.clone();
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap_1()
            .child(
                div()
                    .id(eid("menu", id))
                    .role(accesskit::Role::Button)
                    .aria_label(label.clone())
                    .aria_expanded(open)
                    .cursor_pointer()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(t.surface_alt)
                    .text_sm()
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.toggle_menu(id);
                        cx.notify();
                    })),
            )
            .when(open, |d| {
                d.child(deferred(anchored().snap_to_window().child(self.render_menu_panel(&actions, t, cx))).priority(1))
            })
            .into_any_element()
    }

    /// `<Grid>`: like `<List>`, with a responsive column count instead of
    /// one row per item (SPEC.md §6.3). It reuses the keyed reconciler, so
    /// its children are already the cell elements.
    fn render_grid(&self, node: &Node, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let columns: usize = match self.class {
            WidthClass::Compact => 2,
            WidthClass::Regular => 3,
            WidthClass::Wide => 4,
        };
        let cells = self.render_children(node, t, cx);
        div()
            .grid()
            .grid_cols(columns as u16)
            .gap_3()
            .children(cells.into_iter().map(|c| div().min_w_0().child(c)))
            .into_any_element()
    }
}

/// Applies the `align` prop of Text and Heading (UI API 1.1).
fn aligned<D: Styled>(d: D, node: &Node) -> D {
    match node.enum_prop(prop::ALIGN) {
        text_align::CENTER => d.text_center(),
        text_align::END => d.text_right(),
        _ => d,
    }
}
