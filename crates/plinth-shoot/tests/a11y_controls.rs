//! PRODUCTION U3 and U4: every Level 1 control and every Level 2 primitive,
//! rendered headless, with its AccessKit role, name, value and state
//! (U3), and its keyboard behavior (U4): Tab and Shift+Tab order,
//! Enter and Space, typing, a visible focus ring, Escape, and arrow keys.
//!
//! The fixture app is `CONTROLS` below. `PLINTH_A11Y_DUMP=1` prints the
//! AccessKit tree and the Tab order.

use gpui::accesskit::{Node, NodeId, Rect, Role, Toggled, TreeUpdate};
use gpui::{
    AnyWindowHandle, AppContext as _, HeadlessAppContext, Keystroke, MouseButton, MouseDownEvent, MouseUpEvent, PlatformHeadlessRenderer,
    PlatformInput, Point, WindowHandle, point, px, size,
};
use plinth_compiler::driver::MemFs;
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::{GuestPort, PlinthRoot};

struct WasmGuest {
    guest: Guest,
    _runner: Runner,
}

impl GuestPort for WasmGuest {
    fn dispatch(&mut self, events: &[u8]) -> anyhow::Result<Vec<Vec<u8>>> {
        self.guest.on_event(events)
    }
}

/// One of each control. `log` shows the last thing that happened, so a
/// test can see that a handler ran.
const CONTROLS: &str = r#"
import {
  app, signal, Screen, Section, Group, Tabs, Sheet, Dialog, Menu, Action, List, Row, Grid, Text, Heading,
  Badge, Progress, Empty, Image, Icon, Chart, Button, TextField, TextArea, NumberField, Toggle, Checkbox,
  Picker, Slider, DatePicker, Box, Span, Pressable, Scroll, Canvas, rect,
} from "plinth:ui";

type Item = { id: number; name: string };
const ITEMS: Item[] = [{ id: 1, name: "Apple" }, { id: 2, name: "Pear" }];

const log = signal("none");
const name = signal("");
const notes = signal("");
const count = signal(3);
const wifi = signal(true);
const agree = signal(false);
const size = signal("M");
const city = signal("Oslo");
const volume = signal(40);
const due = signal("2026-10-06");
const tab = signal("One");
const sheet = signal(false);
const dialog = signal(false);

function Home() {
  return (
    <Screen title="Controls" actions={[<Action label="Share" onPress={() => log.set("share")} />]}>
      <Text>{`log: ${log()}`}</Text>
      <Text>{`state: ${name()}|${count()}|${wifi()}|${agree()}|${size()}|${city()}|${volume()}|${tab()}|${due()}|${notes()}`}</Text>
      <Heading level={2}>Inputs</Heading>
      <Section title="Inputs">
        <Button label="Save" role="primary" onPress={() => log.set("save")} />
        <Button label="Locked" disabled={true} onPress={() => log.set("locked")} />
        <TextField label="Name" value={name} placeholder="Your name" />
        <TextArea label="Notes" value={notes} />
        <NumberField label="Count" value={count} min={0} max={10} />
        <Toggle label="Wi-Fi" value={wifi} />
        <Checkbox label="Agree" value={agree} />
        <Picker label="Size" value={size} options={["S", "M", "L"]} />
        <Picker label="City" value={city} options={["Oslo", "Rome", "Lima", "Kyiv", "Pune"]} />
        <Slider label="Volume" value={volume} min={0} max={100} step={10} />
        <DatePicker label="Due" value={due} />
      </Section>
      <Section title="Structure">
        <Tabs items={["One", "Two"]} value={tab} />
        <Group axis="row">
          <Button label="Open sheet" onPress={() => sheet.set(true)} />
          <Button label="Open dialog" onPress={() => dialog.set(true)} />
        </Group>
        <Menu label="More" actions={[<Action label="Rename" onPress={() => log.set("rename")} />, <Action label="Archive" onPress={() => log.set("archive")} />]} />
      </Section>
      <Section title="Collections">
        <List items={ITEMS} key={(i) => i.id} row={(i) => <Row title={i.name} selected={i.id === 1} onPress={() => log.set(i.name)} />} />
        <Grid items={ITEMS} key={(i) => i.id} cell={(i) => <Text>{i.name}</Text>} />
        <Empty title="Nothing here" message="Add an item" />
      </Section>
      <Section title="Content">
        <Badge label="New" />
        <Progress label="Upload" value={0.5} />
        <Image src="pic.png" alt="A test picture" />
        <Icon name="star" label="Favorite" />
        <Chart label="Sales" kind="bar" data={[{ label: "Jan", value: 3 }]} />
      </Section>
      <Section title="Primitives">
        <Box label="Card" padding={2} onKeyDown={(k) => log.set(`key ${k}`)}>
          <Span>Card text</Span>
        </Box>
        <Pressable label="Press box" role="button" padding={2} onPress={() => log.set("press")}>
          <Span>Press</Span>
        </Pressable>
        <Pressable label="Open link" role="link" padding={2} onPress={() => log.set("link")}>
          <Span>Link</Span>
        </Pressable>
        <Scroll label="Strip" direction="row" gap={2}>
          <Span>One</Span>
          <Span>Two</Span>
        </Scroll>
        <Canvas label="A red square" viewWidth={10} viewHeight={10} maxWidth={10} shapes={[rect(0, 0, 10, 10, "danger")]} />
      </Section>
      <Sheet open={sheet} title="Edit" onClose={() => log.set("sheet closed")}>
        <TextField label="Title" value={name} />
        <Button label="Done" onPress={() => sheet.set(false)} />
      </Sheet>
      <Dialog open={dialog} title="Confirm" message="Are you sure?" actions={[<Action label="OK" onPress={() => dialog.set(false)} />]} />
    </Screen>
  );
}

function About() {
  return (
    <Screen title="About">
      <Text>About this app</Text>
    </Screen>
  );
}

export default app({
  screens: {
    home: { title: "Home", icon: "house", component: Home },
    about: { title: "About", icon: "info", component: About },
  },
});
"#;

fn build() -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", CONTROLS).with("assets/pic.png", "not a real picture");
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("the controls fixture has errors:\n{}", diags.join("\n")))
}

/// A headless window with the fixture app and the AccessKit tree on.
struct Harness {
    cx: HeadlessAppContext,
    window: WindowHandle<PlinthRoot>,
    scale: f32,
}

impl Harness {
    fn new() -> Self {
        let artifact = build();
        let platform = gpui_platform::current_platform(true);
        let mut cx = HeadlessAppContext::with_platform(platform.text_system(), std::sync::Arc::new(()), || {
            gpui_wgpu::WgpuHeadlessRenderer::new()
                .map(|r| Box::new(r) as Box<dyn PlatformHeadlessRenderer>)
                .map_err(|e| log::error!("no headless renderer: {e:#}"))
                .ok()
        });
        cx.update(plinth_ui::init);
        let runner = Runner::new().unwrap();
        let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
        let commits = guest.init(&[]).unwrap();
        let port = Box::new(WasmGuest { guest, _runner: runner });
        // Tall, so that the whole screen is in the viewport; 1000 px wide is
        // the regular width class (a navigation rail).
        let window =
            cx.open_window(size(px(1000.), px(3000.)), move |_, cx| cx.new(|cx| PlinthRoot::new(port, commits, "teal", cx))).unwrap();
        cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(true))).unwrap();
        cx.run_until_parked();
        let scale = cx.update(|cx| window.update(cx, |_, window, _| window.scale_factor())).unwrap();
        Self { cx, window, scale }
    }

    fn handle(&self) -> AnyWindowHandle {
        self.window.into()
    }

    fn tree(&mut self) -> TreeUpdate {
        let window = self.window;
        self.cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().expect("a11y tree")
    }

    fn key(&mut self, keys: &str) {
        for k in keys.split(' ') {
            let stroke = Keystroke::parse(k).unwrap();
            self.cx.update_window(self.handle(), |_, window, cx| window.dispatch_keystroke(stroke, cx)).unwrap();
            self.cx.run_until_parked();
        }
    }

    /// Types `text` one character at a time.
    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let stroke = Keystroke::parse(&c.to_string()).unwrap();
            self.cx.update_window(self.handle(), |_, window, cx| window.dispatch_keystroke(stroke, cx)).unwrap();
            self.cx.run_until_parked();
        }
    }

    fn click(&mut self, r: Rect) {
        let pos = point(px(((r.x0 + r.x1) / 2.0) as f32 / self.scale), px(((r.y0 + r.y1) / 2.0) as f32 / self.scale));
        self.click_at(pos);
    }

    fn click_at(&mut self, pos: Point<gpui::Pixels>) {
        self.cx
            .update_window(self.handle(), |_, window, cx| {
                window.dispatch_event(
                    PlatformInput::MouseDown(MouseDownEvent {
                        position: pos,
                        modifiers: Default::default(),
                        button: MouseButton::Left,
                        click_count: 1,
                        first_mouse: false,
                    }),
                    cx,
                );
                window.dispatch_event(
                    PlatformInput::MouseUp(MouseUpEvent { position: pos, modifiers: Default::default(), button: MouseButton::Left, click_count: 1 }),
                    cx,
                );
            })
            .unwrap();
        self.cx.run_until_parked();
    }

    /// The window as `(width, height, RGBA bytes)`, in device pixels.
    fn screenshot(&mut self) -> (u32, u32, Vec<u8>) {
        let handle = self.handle();
        let image = self.cx.capture_screenshot(handle).expect("screenshot");
        (image.width(), image.height(), image.into_raw())
    }
}


fn name(n: &Node) -> String {
    n.label().or(n.value()).unwrap_or("").to_owned()
}

fn node(tree: &TreeUpdate, id: NodeId) -> Option<&Node> {
    tree.nodes.iter().find(|(i, _)| *i == id).map(|(_, n)| n)
}

fn find<'a>(tree: &'a TreeUpdate, role: Role, label: &str) -> Option<(NodeId, &'a Node)> {
    tree.nodes.iter().find(|(_, n)| n.role() == role && n.label() == Some(label)).map(|(id, n)| (*id, n))
}

fn must<'a>(tree: &'a TreeUpdate, role: Role, label: &str) -> &'a Node {
    find(tree, role, label).map(|(_, n)| n).unwrap_or_else(|| panic!("no {role:?} named {label:?} in:\n{}", dump(tree)))
}

fn parent_of(tree: &TreeUpdate, child: NodeId) -> Option<(NodeId, &Node)> {
    tree.nodes.iter().find(|(_, n)| n.children().contains(&child)).map(|(id, n)| (*id, n))
}

/// The name of the nearest ancestor of `id` that has one.
fn ancestor_name(tree: &TreeUpdate, id: NodeId) -> Option<String> {
    let mut cur = id;
    while let Some((pid, p)) = parent_of(tree, cur) {
        if let Some(l) = p.label().filter(|l| !l.is_empty()) {
            return Some(l.to_owned());
        }
        cur = pid;
    }
    None
}

/// `"<role> <name>"`; a node with no name (a text input) is told by the
/// named group around it: `"TextInput in Name"`.
fn desc(tree: &TreeUpdate, id: NodeId) -> String {
    let Some(n) = node(tree, id) else { return "nothing".into() };
    let own = name(n);
    if !own.is_empty() {
        return format!("{:?} {own}", n.role());
    }
    match ancestor_name(tree, id) {
        Some(a) => format!("{:?} in {a}", n.role()),
        None => format!("{:?}", n.role()),
    }
}

fn dump(tree: &TreeUpdate) -> String {
    let mut out = String::new();
    for (id, n) in &tree.nodes {
        out.push_str(&format!(
            "{:>20} {:?} {:?} value={:?} toggled={:?} selected={:?} expanded={:?} disabled={} num={:?} min={:?} max={:?} level={:?} actions:{}{}\n",
            id.0,
            n.role(),
            n.label().unwrap_or(""),
            n.value(),
            n.toggled(),
            n.is_selected(),
            n.is_expanded(),
            n.is_disabled(),
            n.numeric_value(),
            n.min_numeric_value(),
            n.max_numeric_value(),
            n.level(),
            if n.supports_action(gpui::accesskit::Action::Focus) { " focus" } else { "" },
            if n.supports_action(gpui::accesskit::Action::Click) { " click" } else { "" },
        ));
    }
    out
}

impl Harness {
    /// The focused node, as [`desc`] tells it.
    fn now(&mut self) -> String {
        let tree = self.tree();
        desc(&tree, tree.focus)
    }

    /// Presses Tab until `want` has the focus.
    fn tab_to(&mut self, want: &str) {
        let mut seen = Vec::new();
        for _ in 0..80 {
            if self.now() == want {
                return;
            }
            self.key("tab");
            seen.push(self.now());
        }
        panic!("Tab never reached {want:?}; it went through {seen:#?}");
    }

    /// The text of the `state: ...` label: the values of the inputs.
    fn state(&mut self) -> String {
        let tree = self.tree();
        tree.nodes.iter().filter_map(|(_, n)| n.label()).find(|l| l.starts_with("state: ")).unwrap_or("").to_owned()
    }

    fn log(&mut self) -> String {
        let tree = self.tree();
        tree.nodes.iter().filter_map(|(_, n)| n.label()).find(|l| l.starts_with("log: ")).unwrap_or("").to_owned()
    }
}

/// The interactive controls in Tab order (PRODUCTION U4): the navigation,
/// the toolbar, then the screen from the top. A disabled button is not a
/// tab stop; a radio group, a list box and a tab list are one stop each
/// (their chosen option).
const TAB_ORDER: &[&str] = &[
    "Button Controls",
    "Button About",
    "Button Share",
    "Button Save",
    "TextInput in Name",
    "MultilineTextInput in Notes",
    "TextInput in Count",
    "Switch Wi-Fi",
    "CheckBox Agree",
    "RadioButton M",
    "ListBoxOption Oslo",
    "Slider Volume",
    "Button Due: Oct 6, 2026",
    "Tab One",
    "Button Open sheet",
    "Button Open dialog",
    "Button More",
    "ListItem Apple",
    "ListItem Pear",
    "Group Card",
    "Button Press box",
    "Link Open link",
];

#[test]
fn dump_tree_and_tab_order() {
    if std::env::var_os("PLINTH_A11Y_DUMP").is_none() {
        return;
    }
    let mut h = Harness::new();
    let tree = h.tree();
    eprintln!("{}", dump(&tree));
    for _ in 0..30 {
        h.key("tab");
        eprintln!("tab -> {}", h.now());
    }
}

/// U3: each control has a role that fits it, a name, and its value and
/// state.
#[test]
fn every_control_has_a_role_name_value_and_state() {
    let mut h = Harness::new();
    let tree = h.tree();
    let d = || dump(&tree);

    // Structure and content.
    assert_eq!(must(&tree, Role::Heading, "Controls").level(), Some(1), "the screen title is a level 1 heading");
    assert_eq!(must(&tree, Role::Heading, "Inputs").level(), Some(2), "a Heading has its level");
    must(&tree, Role::Navigation, "Screens");
    must(&tree, Role::Button, "Controls");
    must(&tree, Role::Button, "About");
    must(&tree, Role::Group, "Inputs");
    must(&tree, Role::Label, "log: none");
    must(&tree, Role::Label, "New");
    must(&tree, Role::Label, "Nothing here");
    must(&tree, Role::Label, "Add an item");
    must(&tree, Role::Image, "A test picture");
    must(&tree, Role::Image, "Favorite");
    must(&tree, Role::Figure, "Sales");
    let progress = must(&tree, Role::ProgressIndicator, "Upload");
    assert_eq!((progress.numeric_value(), progress.min_numeric_value(), progress.max_numeric_value()), (Some(0.5), Some(0.0), Some(1.0)));

    // Buttons and actions.
    let save = must(&tree, Role::Button, "Save");
    assert!(save.supports_action(gpui::accesskit::Action::Focus) && save.supports_action(gpui::accesskit::Action::Click));
    let locked = must(&tree, Role::Button, "Locked");
    assert!(locked.is_disabled() && !locked.supports_action(gpui::accesskit::Action::Focus), "a disabled button is not a tab stop:\n{}", d());
    must(&tree, Role::Button, "Share");
    assert_eq!(must(&tree, Role::Button, "More").is_expanded(), Some(false), "a Menu trigger tells that it is closed");

    // Text input: the input is inside a group named by the label.
    for (role, label) in [(Role::TextInput, "Name"), (Role::MultilineTextInput, "Notes"), (Role::TextInput, "Count")] {
        let (id, _) = tree
            .nodes
            .iter()
            .find(|(id, n)| n.role() == role && ancestor_name(&tree, *id).as_deref() == Some(label))
            .unwrap_or_else(|| panic!("no {role:?} in a group named {label:?}:\n{}", d()));
        // gpui-ce writes no Focus action (and no name) on a text input node;
        // the Tab order test shows that the input takes the focus.
        let _ = id;
    }
    let count = must(&tree, Role::SpinButton, "Count");
    assert_eq!((count.numeric_value(), count.min_numeric_value(), count.max_numeric_value()), (Some(3.0), Some(0.0), Some(10.0)));

    // Choices.
    assert_eq!(must(&tree, Role::Switch, "Wi-Fi").toggled(), Some(Toggled::True));
    assert_eq!(must(&tree, Role::CheckBox, "Agree").toggled(), Some(Toggled::False));
    must(&tree, Role::RadioGroup, "Size");
    assert_eq!(must(&tree, Role::RadioButton, "M").toggled(), Some(Toggled::True));
    assert_eq!(must(&tree, Role::RadioButton, "S").toggled(), Some(Toggled::False));
    must(&tree, Role::ListBox, "City");
    assert_eq!(must(&tree, Role::ListBoxOption, "Oslo").is_selected(), Some(true));
    assert_eq!(must(&tree, Role::ListBoxOption, "Rome").is_selected(), Some(false));
    let volume = must(&tree, Role::Slider, "Volume");
    assert_eq!((volume.numeric_value(), volume.min_numeric_value(), volume.max_numeric_value()), (Some(40.0), Some(0.0), Some(100.0)));
    must(&tree, Role::Button, "Decrease Volume");
    assert_eq!(must(&tree, Role::Button, "Due: Oct 6, 2026").is_expanded(), Some(false));
    assert_eq!(must(&tree, Role::Tab, "One").is_selected(), Some(true));
    assert_eq!(must(&tree, Role::Tab, "Two").is_selected(), Some(false));

    // Collections.
    assert!(tree.nodes.iter().any(|(_, n)| n.role() == Role::List), "a List is a list:\n{}", d());
    let apple = must(&tree, Role::ListItem, "Apple");
    assert_eq!(apple.is_selected(), Some(true), "the selected row tells it");
    assert!(apple.supports_action(gpui::accesskit::Action::Focus), "a row with onPress takes the focus");

    // Level 2 primitives.
    let card = must(&tree, Role::Group, "Card");
    assert!(card.supports_action(gpui::accesskit::Action::Focus), "a Box with a key handler is a tab stop");
    for (role, label) in [(Role::Button, "Press box"), (Role::Link, "Open link")] {
        let n = must(&tree, role, label);
        assert!(n.supports_action(gpui::accesskit::Action::Focus), "a Pressable with onPress takes the focus: {label}");
        assert!(n.supports_action(gpui::accesskit::Action::Click), "a Pressable with onPress can be pressed: {label}");
    }
    must(&tree, Role::Label, "Card text");
    must(&tree, Role::ScrollView, "Strip");
    must(&tree, Role::Image, "A red square");

    // Each node that takes the focus or a click has a name (its own, or
    // that of the group around a text input).
    for (id, n) in &tree.nodes {
        let acts = n.supports_action(gpui::accesskit::Action::Focus) || n.supports_action(gpui::accesskit::Action::Click);
        if acts && *id != NodeId(0) {
            let named = !name(n).is_empty() || ancestor_name(&tree, *id).is_some();
            assert!(named, "a {:?} that takes the focus or a click has no name:\n{}", n.role(), d());
        }
    }
}

/// U4: Tab and Shift+Tab go through every interactive control in order.
#[test]
fn tab_and_shift_tab_reach_every_control_in_order() {
    let mut h = Harness::new();
    let mut forward = Vec::new();
    for _ in 0..TAB_ORDER.len() + 1 {
        h.key("tab");
        forward.push(h.now());
    }
    let want: Vec<String> = TAB_ORDER.iter().chain(&TAB_ORDER[..1]).map(|s| s.to_string()).collect();
    assert_eq!(forward, want, "the Tab order (it goes around at the end)");

    // Shift+Tab goes back, around the start too.
    let mut back = Vec::new();
    for _ in 0..TAB_ORDER.len() {
        h.key("shift-tab");
        back.push(h.now());
    }
    let want: Vec<String> = TAB_ORDER.iter().rev().map(|s| s.to_string()).collect();
    assert_eq!(back, want, "the Shift+Tab order");
}

/// U4: Enter and Space press buttons, rows, actions and pressables, and
/// flip switches and checkboxes; text fields take typing and give Tab on.
#[test]
fn enter_space_and_typing_work() {
    let mut h = Harness::new();
    h.tab_to("Button Save");
    h.key("enter");
    assert_eq!(h.log(), "log: save", "Enter presses a Button");
    h.tab_to("Button Share");
    h.key("space");
    assert_eq!(h.log(), "log: share", "Space presses a toolbar Action");
    h.tab_to("ListItem Pear");
    h.key("enter");
    assert_eq!(h.log(), "log: Pear", "Enter presses a Row");
    h.tab_to("Button Press box");
    h.key("space");
    assert_eq!(h.log(), "log: press", "Space presses a Pressable button");
    h.tab_to("Link Open link");
    h.key("enter");
    assert_eq!(h.log(), "log: link", "Enter presses a Pressable link");
    h.tab_to("Group Card");
    h.key("k");
    assert_eq!(h.log(), "log: key k", "a Box with onKeyDown gets the keys");

    h.tab_to("Switch Wi-Fi");
    h.key("space");
    assert!(h.state().contains("|3|false|"), "Space flips a Toggle: {}", h.state());
    h.tab_to("CheckBox Agree");
    h.key("space");
    assert!(h.state().contains("|false|true|"), "Space flips a Checkbox: {}", h.state());

    h.tab_to("TextInput in Name");
    h.type_text("Ann");
    assert!(h.state().starts_with("state: Ann|"), "a TextField takes typing: {}", h.state());
    h.tab_to("MultilineTextInput in Notes");
    h.type_text("hi");
    h.key("tab");
    assert_eq!(h.now(), "TextInput in Count", "Tab leaves a TextArea");
    assert!(h.state().ends_with("|hi"), "Tab types no tab character: {:?}", h.state());

    // Enter on a navigation button shows that screen.
    h.tab_to("Button About");
    h.key("enter");
    let tree = h.tree();
    must(&tree, Role::Heading, "About");
}

/// U4: arrow keys move in a Picker, Tabs, a Slider, a NumberField and the
/// DatePicker grid; the focus follows the choice.
#[test]
fn arrow_keys_move_in_choices_and_values() {
    let mut h = Harness::new();
    h.tab_to("RadioButton M");
    h.key("right");
    assert_eq!(h.now(), "RadioButton L");
    assert!(h.state().contains("|L|"), "Right chooses the next option: {}", h.state());
    h.key("right");
    assert_eq!(h.now(), "RadioButton S", "the arrows go around the ends");
    h.key("left");
    assert_eq!(h.now(), "RadioButton L");

    h.key("tab");
    assert_eq!(h.now(), "ListBoxOption Oslo");
    h.key("down");
    assert_eq!(h.now(), "ListBoxOption Rome");
    assert!(h.state().contains("|Rome|"), "{}", h.state());
    h.key("end");
    assert_eq!(h.now(), "ListBoxOption Pune");

    h.key("tab");
    assert_eq!(h.now(), "Slider Volume");
    h.key("right");
    assert!(h.state().contains("|50|"), "Right steps a Slider: {}", h.state());
    h.key("home");
    assert!(h.state().contains("|0|"), "Home goes to the min: {}", h.state());
    h.key("end");
    assert!(h.state().contains("|100|"), "End goes to the max: {}", h.state());

    h.tab_to("Tab One");
    h.key("right");
    assert_eq!(h.now(), "Tab Two");
    assert!(h.state().contains("|Two|"), "{}", h.state());

    h.tab_to("TextInput in Count");
    h.key("up");
    assert!(h.state().contains("|4|"), "Up steps a NumberField: {}", h.state());
    h.key("down down");
    assert!(h.state().contains("|2|"), "Down steps a NumberField: {}", h.state());

    // DatePicker: Enter opens the grid with the focus in it; the arrows
    // move the day; Enter picks it, closes, and gives the focus back.
    h.tab_to("Button Due: Oct 6, 2026");
    h.key("enter");
    let tree = h.tree();
    assert!(tree.nodes.iter().any(|(_, n)| n.role() == Role::Grid), "Enter opens the grid:\n{}", dump(&tree));
    h.key("right");
    assert_eq!(h.now(), "GridCell Oct 7, 2026", "the focused day is the active one");
    h.key("enter");
    assert!(h.state().contains("|2026-10-07|"), "Enter picks the day: {}", h.state());
    assert_eq!(h.now(), "Button Due: Oct 7, 2026", "the focus goes back to the trigger");
    h.key("enter");
    h.key("escape");
    let tree = h.tree();
    assert!(!tree.nodes.iter().any(|(_, n)| n.role() == Role::Grid), "Escape closes the grid");
    assert_eq!(h.now(), "Button Due: Oct 7, 2026");
}

/// U4: a Menu opens with Enter, Up and Down move in it, Escape closes it
/// and gives the focus back; Sheet and Dialog keep the focus in them and
/// close on Escape.
#[test]
fn escape_closes_popovers_and_modals() {
    let mut h = Harness::new();
    h.tab_to("Button More");
    h.key("enter");
    assert_eq!(h.now(), "MenuItem Rename", "an open menu has the focus on its first item");
    h.key("down");
    assert_eq!(h.now(), "MenuItem Archive");
    h.key("down");
    assert_eq!(h.now(), "MenuItem Rename", "Down goes around");
    h.key("escape");
    let tree = h.tree();
    assert!(!tree.nodes.iter().any(|(_, n)| n.role() == Role::Menu), "Escape closes the menu");
    assert_eq!(h.now(), "Button More", "the focus goes back to the trigger");
    h.key("space down enter");
    assert_eq!(h.log(), "log: archive", "Enter presses a menu item");
    assert_eq!(h.now(), "Button More");

    // Sheet: the focus goes in, Tab stays in, Escape closes it.
    h.tab_to("Button Open sheet");
    h.key("enter");
    let mut inside = Vec::new();
    for _ in 0..4 {
        h.key("tab");
        inside.push(h.now());
    }
    assert_eq!(inside, ["Button Close", "TextInput in Title", "Button Done", "Button Close"], "Tab stays in the Sheet");
    h.key("shift-tab");
    assert_eq!(h.now(), "Button Done", "Shift+Tab stays in the Sheet");
    h.key("shift-tab");
    h.key("escape");
    assert_eq!(h.log(), "log: sheet closed", "Escape in its text field closes the Sheet");
    assert_eq!(h.now(), "Button Open sheet", "the focus goes back to the opener");

    // Dialog.
    h.tab_to("Button Open dialog");
    h.key("enter");
    let tree = h.tree();
    assert!(must(&tree, Role::Dialog, "Confirm").is_modal());
    h.key("tab");
    assert_eq!(h.now(), "Button OK");
    h.key("tab");
    assert_eq!(h.now(), "Button OK", "Tab stays in the Dialog");
    h.key("escape");
    let tree = h.tree();
    assert!(find(&tree, Role::Dialog, "Confirm").is_none(), "Escape closes the Dialog");
    assert_eq!(h.now(), "Button Open dialog");
}

/// U4: a control with the keyboard focus has a visible ring: the pixels
/// just outside its bounds change when it gets the focus.
#[test]
fn focused_controls_draw_a_focus_ring() {
    let mut h = Harness::new();
    let (w, _, before) = h.screenshot();
    for want in [
        "Button Save",
        "TextInput in Name",
        "Switch Wi-Fi",
        "RadioButton M",
        "ListBoxOption Oslo",
        "Slider Volume",
        "Tab One",
        "ListItem Apple",
        "Group Card",
        "Button Press box",
        "Button Controls",
    ] {
        h.tab_to(want);
        let tree = h.tree();
        let r = node(&tree, tree.focus).and_then(|n| n.bounds()).unwrap_or_else(|| panic!("{want} has no bounds"));
        let (_, _, after) = h.screenshot();
        // A band 2 device pixels outside the bounds (a ring) or inside them
        // (an inset ring, on rows), along the top and the bottom edge, not
        // near the corners: most of its pixels change.
        let (x0, x1) = (r.x0.round() as i64 + 6, r.x1.round() as i64 - 6);
        let band = |ys: [i64; 2]| {
            let (mut all, mut changed) = (0, 0);
            for y in ys {
                for x in x0..x1 {
                    let i = ((y * w as i64 + x) * 4) as usize;
                    all += 1;
                    if before[i..i + 3] != after[i..i + 3] {
                        changed += 1;
                    }
                }
            }
            (changed, all)
        };
        let (y0, y1) = (r.y0.round() as i64, r.y1.round() as i64);
        let (out, all) = band([y0 - 2, y1 + 1]);
        let (inside, _) = band([y0 + 1, y1 - 2]);
        // 70 %: on a round control (a Toggle) the ring follows the curve
        // near the ends.
        assert!(out * 10 > all * 7 || inside * 10 > all * 7, "{want}: no focus ring ({out} outside, {inside} inside, of {all} pixels changed)");
    }
}
