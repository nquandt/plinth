//! The props of each UI API 1.0 control (SPEC.md §6.3), as the compiler
//! checks them. Keep this table in sync with `std/ui.d.ts`; the test
//! `std_typings_match` compares them.

use plinth_protocol::{ControlKind, button_role, event, prop, text_style, tone};

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
    /// `Signal<string> | string`
    ValueStr,
    /// `Signal<boolean> | boolean`
    ValueBool,
    ListItems,
    ListKey,
    ListRow,
    Element,
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

pub const ICONS: &[&str] = &["house", "gear", "check", "list", "plus", "trash", "star", "info", "number"];

const BUTTON_ROLES: &[(&str, u16)] =
    &[("default", button_role::DEFAULT), ("primary", button_role::PRIMARY), ("destructive", button_role::DESTRUCTIVE)];
const TEXT_STYLES: &[(&str, u16)] = &[("body", text_style::BODY), ("caption", text_style::CAPTION), ("mono", text_style::MONO)];
const TONES: &[(&str, u16)] =
    &[("default", tone::DEFAULT), ("muted", tone::MUTED), ("danger", tone::DANGER), ("success", tone::SUCCESS)];

const fn p(name: &'static str, ty: PropTy, required: bool, target: Target) -> PropSpec {
    PropSpec { name, ty, required, target }
}

use PropTy as T;
use Target::{Event as Ev, Prop as P};

pub const CONTROLS: &[ControlSpec] = &[
    ControlSpec {
        name: "Screen",
        kind: ControlKind::Screen,
        props: &[p("title", T::Str, true, P(prop::TITLE))],
        children: ChildKind::Nodes,
    },
    ControlSpec {
        name: "Section",
        kind: ControlKind::Section,
        props: &[p("title", T::Str, false, P(prop::TITLE)), p("footer", T::Str, false, P(prop::FOOTER))],
        children: ChildKind::Nodes,
    },
    ControlSpec {
        name: "Text",
        kind: ControlKind::Text,
        props: &[p("style", T::Enum(TEXT_STYLES), false, P(prop::STYLE)), p("tone", T::Enum(TONES), false, P(prop::TONE))],
        children: ChildKind::Text,
    },
    ControlSpec {
        name: "Heading",
        kind: ControlKind::Heading,
        props: &[p("level", T::Int, false, P(prop::LEVEL))],
        children: ChildKind::Text,
    },
    ControlSpec {
        name: "Button",
        kind: ControlKind::Button,
        props: &[
            p("label", T::Str, true, P(prop::LABEL)),
            p("role", T::Enum(BUTTON_ROLES), false, P(prop::ROLE)),
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
];

pub fn by_name(name: &str) -> Option<&'static ControlSpec> {
    CONTROLS.iter().find(|c| c.name == name)
}

pub fn by_kind(kind: ControlKind) -> &'static ControlSpec {
    CONTROLS.iter().find(|c| c.kind == kind).expect("every control kind has a spec")
}

pub const ACCENTS: &[&str] = &["teal", "blue", "indigo", "purple", "pink", "red", "orange", "green"];
