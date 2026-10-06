//! The props of each UI API 1.0 control (SPEC.md §6.3), as the compiler
//! checks them. Keep this table in sync with `std/ui.d.ts`; the test
//! `std_typings_match` compares them.

use plinth_protocol::{ControlKind, aspect, axis, button_role, button_size, event, prop, text_align, text_style, tone};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropTy {
    Str,
    /// A string from a fixed set (icon names).
    StrOneOf(&'static [&'static str]),
    Num,
    /// A number sent as an int prop (`level`).
    Int,
    Bool,
    Enum(&'static [(&'static str, u16)]),
    /// `() => void`
    Callback0,
    /// `(value: string) => void`
    CallbackStr,
    /// `(value: boolean) => void`
    CallbackBool,
    /// `(value: number) => void`
    CallbackNum,
    /// `Signal<string> | string`
    ValueStr,
    /// `Signal<boolean> | boolean`
    ValueBool,
    /// `Signal<number> | number`
    ValueNum,
    /// A literal array of string literals (`Picker.options`, `Tabs.items`),
    /// joined with U+001F into one string prop. Not reactive.
    StrList,
    ListItems,
    ListKey,
    ListRow,
    Element,
    /// A literal array of `<Action>` elements (UI API 1.2, `Screen`/`Dialog`/
    /// `Menu` `actions`). The elements become child nodes; the renderer
    /// tells them apart from body children by their control kind.
    ActionList,
    /// `Image.src`: a string literal that must name a file under `assets/`
    /// (UI API 1.3, SPEC.md §6.3).
    Asset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Prop(u16),
    Event(u16),
    /// `value`: a prop, or a two-way binding when the value is a signal.
    Value,
    List,
}

#[derive(Debug, Clone, Copy)]
pub struct PropSpec {
    pub name: &'static str,
    pub ty: PropTy,
    pub required: bool,
    pub target: Target,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildKind {
    None,
    Text,
    Nodes,
}

pub struct ControlSpec {
    pub name: &'static str,
    pub kind: ControlKind,
    pub props: &'static [PropSpec],
    pub children: ChildKind,
}

pub const ICONS: &[&str] = &[
    "house", "gear", "check", "list", "plus", "trash", "star", "info", "number",
    // UI API 1.2
    "search", "edit", "close", "back", "forward", "calendar", "clock", "user", "mail", "heart", "bell", "share",
    "download", "upload", "refresh", "filter", "menu", "more", "lock", "warning",
];

const BUTTON_ROLES: &[(&str, u16)] =
    &[("default", button_role::DEFAULT), ("primary", button_role::PRIMARY), ("destructive", button_role::DESTRUCTIVE)];
const TEXT_STYLES: &[(&str, u16)] = &[("body", text_style::BODY), ("caption", text_style::CAPTION), ("mono", text_style::MONO)];
const AXES: &[(&str, u16)] = &[("auto", axis::AUTO), ("row", axis::ROW), ("column", axis::COLUMN)];
const BUTTON_SIZES: &[(&str, u16)] = &[("default", button_size::DEFAULT), ("large", button_size::LARGE)];
const ALIGNS: &[(&str, u16)] = &[("start", text_align::START), ("center", text_align::CENTER), ("end", text_align::END)];
const TONES: &[(&str, u16)] =
    &[("default", tone::DEFAULT), ("muted", tone::MUTED), ("danger", tone::DANGER), ("success", tone::SUCCESS)];
const ASPECTS: &[(&str, u16)] = &[("square", aspect::SQUARE), ("wide", aspect::WIDE), ("tall", aspect::TALL)];

const fn p(name: &'static str, ty: PropTy, required: bool, target: Target) -> PropSpec {
    PropSpec { name, ty, required, target }
}

use PropTy as T;
use Target::{Event as Ev, Prop as P};

pub const CONTROLS: &[ControlSpec] = &[
    ControlSpec {
        name: "Screen",
        kind: ControlKind::Screen,
        props: &[p("title", T::Str, true, P(prop::TITLE)), p("actions", T::ActionList, false, Target::List)],
        children: ChildKind::Nodes,
    },
    ControlSpec {
        name: "Section",
        kind: ControlKind::Section,
        props: &[p("title", T::Str, false, P(prop::TITLE)), p("footer", T::Str, false, P(prop::FOOTER))],
        children: ChildKind::Nodes,
    },
    ControlSpec {
        name: "Group",
        kind: ControlKind::Group,
        props: &[p("axis", T::Enum(AXES), false, P(prop::AXIS))],
        children: ChildKind::Nodes,
    },
    ControlSpec {
        name: "Text",
        kind: ControlKind::Text,
        props: &[
            p("style", T::Enum(TEXT_STYLES), false, P(prop::STYLE)),
            p("tone", T::Enum(TONES), false, P(prop::TONE)),
            p("align", T::Enum(ALIGNS), false, P(prop::ALIGN)),
        ],
        children: ChildKind::Text,
    },
    ControlSpec {
        name: "Heading",
        kind: ControlKind::Heading,
        props: &[p("level", T::Int, false, P(prop::LEVEL)), p("align", T::Enum(ALIGNS), false, P(prop::ALIGN))],
        children: ChildKind::Text,
    },
    ControlSpec {
        name: "Button",
        kind: ControlKind::Button,
        props: &[
            p("label", T::Str, true, P(prop::LABEL)),
            p("role", T::Enum(BUTTON_ROLES), false, P(prop::ROLE)),
            p("size", T::Enum(BUTTON_SIZES), false, P(prop::SIZE)),
            p("onPress", T::Callback0, true, Ev(event::PRESS)),
            p("disabled", T::Bool, false, P(prop::DISABLED)),
        ],
        children: ChildKind::None,
    },
    ControlSpec {
        name: "TextField",
        kind: ControlKind::TextField,
        props: &[
            p("label", T::Str, true, P(prop::LABEL)),
            p("value", T::ValueStr, true, Target::Value),
            p("placeholder", T::Str, false, P(prop::PLACEHOLDER)),
            p("error", T::Str, false, P(prop::ERROR)),
            p("onChange", T::CallbackStr, false, Ev(event::CHANGE)),
            p("onSubmit", T::Callback0, false, Ev(event::SUBMIT)),
        ],
        children: ChildKind::None,
    },
    ControlSpec {
        name: "Toggle",
        kind: ControlKind::Toggle,
        props: &[
            p("label", T::Str, true, P(prop::LABEL)),
            p("value", T::ValueBool, true, Target::Value),
            p("onChange", T::CallbackBool, false, Ev(event::CHANGE)),
            p("disabled", T::Bool, false, P(prop::DISABLED)),
        ],
        children: ChildKind::None,
    },
    ControlSpec {
        name: "List",
        kind: ControlKind::List,
        props: &[
            p("items", T::ListItems, true, Target::List),
            p("key", T::ListKey, true, Target::List),
            p("row", T::ListRow, true, Target::List),
            p("empty", T::Element, false, Target::List),
        ],
        children: ChildKind::None,
    },
    ControlSpec {
        name: "Row",
        kind: ControlKind::Row,
        props: &[
            p("title", T::Str, true, P(prop::TITLE)),
            p("subtitle", T::Str, false, P(prop::SUBTITLE)),
            p("icon", T::StrOneOf(ICONS), false, P(prop::ICON)),
            p("onPress", T::Callback0, false, Ev(event::PRESS)),
        ],
        children: ChildKind::Nodes,
    },
    ControlSpec {
        name: "Empty",
        kind: ControlKind::Empty,
        props: &[p("title", T::Str, true, P(prop::TITLE)), p("message", T::Str, false, P(prop::MESSAGE))],
        children: ChildKind::None,
    },
    // -- UI API 1.2 inputs --
    ControlSpec {
        name: "Checkbox",
        kind: ControlKind::Checkbox,
        props: &[
            p("label", T::Str, true, P(prop::LABEL)),
            p("value", T::ValueBool, true, Target::Value),
            p("onChange", T::CallbackBool, false, Ev(event::CHANGE)),
            p("disabled", T::Bool, false, P(prop::DISABLED)),
        ],
        children: ChildKind::None,
    },
    ControlSpec {
        name: "TextArea",
        kind: ControlKind::TextArea,
        props: &[
            p("label", T::Str, true, P(prop::LABEL)),
            p("value", T::ValueStr, true, Target::Value),
            p("placeholder", T::Str, false, P(prop::PLACEHOLDER)),
            p("onChange", T::CallbackStr, false, Ev(event::CHANGE)),
        ],
        children: ChildKind::None,
    },
    ControlSpec {
        name: "Slider",
        kind: ControlKind::Slider,
        props: &[
            p("label", T::Str, true, P(prop::LABEL)),
            p("value", T::ValueNum, true, Target::Value),
            p("min", T::Num, true, P(prop::MIN)),
            p("max", T::Num, true, P(prop::MAX)),
            p("step", T::Num, false, P(prop::STEP)),
            p("onChange", T::CallbackNum, false, Ev(event::CHANGE)),
            p("disabled", T::Bool, false, P(prop::DISABLED)),
        ],
        children: ChildKind::None,
    },
    ControlSpec {
        name: "NumberField",
        kind: ControlKind::NumberField,
        props: &[
            p("label", T::Str, true, P(prop::LABEL)),
            p("value", T::ValueNum, true, Target::Value),
            p("min", T::Num, false, P(prop::MIN)),
            p("max", T::Num, false, P(prop::MAX)),
            p("step", T::Num, false, P(prop::STEP)),
            p("onChange", T::CallbackNum, false, Ev(event::CHANGE)),
            p("disabled", T::Bool, false, P(prop::DISABLED)),
        ],
        children: ChildKind::None,
    },
    ControlSpec {
        name: "Picker",
        kind: ControlKind::Picker,
        props: &[
            p("label", T::Str, true, P(prop::LABEL)),
            p("value", T::ValueStr, true, Target::Value),
            p("options", T::StrList, true, P(prop::OPTIONS)),
            p("onChange", T::CallbackStr, false, Ev(event::CHANGE)),
            p("disabled", T::Bool, false, P(prop::DISABLED)),
        ],
        children: ChildKind::None,
    },
    ControlSpec {
        name: "Progress",
        kind: ControlKind::Progress,
        props: &[
            p("label", T::Str, false, P(prop::LABEL)),
            // Missing (not passed) means indeterminate.
            p("value", T::Num, false, P(prop::VALUE)),
        ],
        children: ChildKind::None,
    },
    ControlSpec {
        name: "Badge",
        kind: ControlKind::Badge,
        props: &[
            p("label", T::Str, true, P(prop::LABEL)),
            p("tone", T::Enum(TONES), false, P(prop::TONE)),
        ],
        children: ChildKind::None,
    },
    // -- UI API 1.2: structure and stack navigation ------------------------
    ControlSpec {
        name: "Tabs",
        kind: ControlKind::Tabs,
        props: &[
            p("items", T::StrList, true, P(prop::ITEMS)),
            p("value", T::ValueStr, true, Target::Value),
            p("onChange", T::CallbackStr, false, Ev(event::CHANGE)),
        ],
        children: ChildKind::None,
    },
    ControlSpec {
        name: "Sheet",
        kind: ControlKind::Sheet,
        props: &[
            p("open", T::ValueBool, true, Target::Value),
            p("title", T::Str, true, P(prop::TITLE)),
            p("onClose", T::Callback0, false, Ev(event::CLOSE)),
        ],
        children: ChildKind::Nodes,
    },
    ControlSpec {
        name: "Dialog",
        kind: ControlKind::Dialog,
        props: &[
            p("open", T::ValueBool, true, Target::Value),
            p("title", T::Str, true, P(prop::TITLE)),
            p("message", T::Str, false, P(prop::MESSAGE)),
            p("actions", T::ActionList, true, Target::List),
        ],
        children: ChildKind::Nodes,
    },
    ControlSpec {
        name: "Menu",
        kind: ControlKind::Menu,
        props: &[p("label", T::Str, true, P(prop::LABEL)), p("actions", T::ActionList, true, Target::List)],
        children: ChildKind::Nodes,
    },
    ControlSpec {
        name: "Grid",
        kind: ControlKind::Grid,
        props: &[
            p("items", T::ListItems, true, Target::List),
            p("key", T::ListKey, true, Target::List),
            p("cell", T::ListRow, true, Target::List),
            p("empty", T::Element, false, Target::List),
        ],
        children: ChildKind::None,
    },
    ControlSpec {
        name: "Action",
        kind: ControlKind::Action,
        props: &[
            p("label", T::Str, true, P(prop::LABEL)),
            p("onPress", T::Callback0, true, Ev(event::PRESS)),
            p("icon", T::StrOneOf(ICONS), false, P(prop::ICON)),
            p("role", T::Enum(BUTTON_ROLES), false, P(prop::ROLE)),
            p("confirm", T::Bool, false, P(prop::CONFIRM)),
        ],
        children: ChildKind::None,
    },
    // -- UI API 1.3 --
    ControlSpec {
        name: "Image",
        kind: ControlKind::Image,
        props: &[
            p("src", T::Asset, true, P(prop::SRC)),
            p("alt", T::Str, true, P(prop::ALT)),
            p("aspect", T::Enum(ASPECTS), false, P(prop::ASPECT)),
        ],
        children: ChildKind::None,
    },
];

pub fn by_name(name: &str) -> Option<&'static ControlSpec> {
    CONTROLS.iter().find(|c| c.name == name)
}

pub fn by_kind(kind: ControlKind) -> &'static ControlSpec {
    CONTROLS.iter().find(|c| c.kind == kind).expect("every control kind has a spec")
}

pub const ACCENTS: &[&str] = &["teal", "blue", "indigo", "purple", "pink", "red", "orange", "green"];
