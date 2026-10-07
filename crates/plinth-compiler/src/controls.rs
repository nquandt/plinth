//! The props of each UI API 1.0 control (SPEC.md §6.3), as the compiler
//! checks them. Keep this table in sync with `std/ui.d.ts`; the test
//! `std_typings_match` compares them.

use plinth_protocol::{
    ControlKind, aspect, axis, button_role, button_size, chart_kind, color, cross_align, date_picker_mode, event, fraction, justify,
    position, pressable_role, prop, radius, text_align, text_size, text_style, tone, weight,
};

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
    /// `Chart.data`: an array literal of `{ label: string, value: number }`
    /// object literals. Encoded at compile time into one `data` string prop
    /// (`"label\u0001value"` pairs joined with U+001F); the `value`
    /// expressions can read signals, so the prop stays reactive like any
    /// other (UI API 1.5, SPEC.md §6.3).
    ChartPoints,
    /// `Chart.series`: an array literal of `{ name: string, points: ... }`
    /// object literals, each `points` an array literal in the same shape
    /// as `Chart.data`. Encoded into one `series` string prop: series
    /// joined with U+001E, each `"name\u0001points"` (UI API 1.5).
    ChartSeries,
    /// A Level 2 size (UI API 1.6): a number of spacing units, sent as an
    /// int prop (the target id), or a fraction string literal (`"1/2"`,
    /// `"full"`, `"auto"`, ...), sent as an enum prop with this id.
    Size(u16),
    /// A Level 2 partial style (UI API 1.7): an object literal whose keys
    /// are props of this list and whose values are literals. Encoded at
    /// compile time into one string prop, `"<prop id>:<int>"` pairs joined
    /// with `,` (`hover={{ bg: "hover" }}`).
    PartialStyle(&'static [PropSpec]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Prop(u16),
    Event(u16),
    /// `value`: a prop, or a two-way binding when the value is a signal.
    Value,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
const DATE_PICKER_MODES: &[(&str, u16)] =
    &[("date", date_picker_mode::DATE), ("time", date_picker_mode::TIME), ("datetime", date_picker_mode::DATETIME)];
const CHART_KINDS: &[(&str, u16)] = &[("bar", chart_kind::BAR), ("line", chart_kind::LINE), ("pie", chart_kind::PIE)];

// -- UI API 1.6: Level 2 style props (docs/UI-ADVANCED.md) --
const DIRECTIONS: &[(&str, u16)] = &[("row", axis::ROW), ("column", axis::COLUMN)];
const CROSS_ALIGNS: &[(&str, u16)] =
    &[("stretch", cross_align::STRETCH), ("start", cross_align::START), ("center", cross_align::CENTER), ("end", cross_align::END)];
const JUSTIFIES: &[(&str, u16)] =
    &[("start", justify::START), ("center", justify::CENTER), ("end", justify::END), ("between", justify::BETWEEN)];
pub const FRACTIONS: &[(&str, u16)] = &[
    ("auto", fraction::AUTO),
    ("full", fraction::FULL),
    ("1/2", fraction::HALF),
    ("1/3", fraction::THIRD),
    ("2/3", fraction::TWO_THIRDS),
    ("1/4", fraction::QUARTER),
    ("3/4", fraction::THREE_QUARTERS),
];
const COLORS: &[(&str, u16)] = &[
    ("none", color::NONE),
    ("background", color::BACKGROUND),
    ("surface", color::SURFACE),
    ("surface.alt", color::SURFACE_ALT),
    ("accent", color::ACCENT),
    ("danger", color::DANGER),
    ("success", color::SUCCESS),
    ("text", color::TEXT),
    ("text.muted", color::TEXT_MUTED),
    ("on.accent", color::ON_ACCENT),
    ("border", color::BORDER),
    ("hover", color::HOVER),
    ("selected", color::SELECTED),
];
const RADII: &[(&str, u16)] = &[("none", radius::NONE), ("sm", radius::SM), ("md", radius::MD), ("lg", radius::LG), ("full", radius::FULL)];
const TEXT_SIZES: &[(&str, u16)] = &[
    ("xs", text_size::XS),
    ("sm", text_size::SM),
    ("md", text_size::MD),
    ("lg", text_size::LG),
    ("xl", text_size::XL),
    ("2xl", text_size::XXL),
];
const WEIGHTS: &[(&str, u16)] =
    &[("regular", weight::REGULAR), ("medium", weight::MEDIUM), ("semibold", weight::SEMIBOLD), ("bold", weight::BOLD)];
const PRESSABLE_ROLES: &[(&str, u16)] = &[("button", pressable_role::BUTTON), ("link", pressable_role::LINK)];
const POSITIONS: &[(&str, u16)] = &[("relative", position::RELATIVE), ("absolute", position::ABSOLUTE)];

/// The props of a Level 2 box: the props of the control itself, the style
/// props, and the partial styles (states and width classes). `@style`
/// gives only the style props.
macro_rules! box_props {
    (@style) => { box_props!(@list []) };
    ($($extra:expr),* $(,)?) => {
        box_props!(@list [
            $($extra,)*
            p("hover", T::PartialStyle(BOX_STYLE), false, P(prop::HOVER)),
            p("active", T::PartialStyle(BOX_STYLE), false, P(prop::ACTIVE)),
            p("focus", T::PartialStyle(BOX_STYLE), false, P(prop::FOCUS)),
            p("compact", T::PartialStyle(BOX_STYLE), false, P(prop::COMPACT)),
            p("regular", T::PartialStyle(BOX_STYLE), false, P(prop::REGULAR)),
            p("wide", T::PartialStyle(BOX_STYLE), false, P(prop::WIDE)),
        ])
    };
    (@list [$($extra:expr),* $(,)?]) => {
        &[
            $($extra,)*
            p("direction", T::Enum(DIRECTIONS), false, P(prop::AXIS)),
            p("wrap", T::Bool, false, P(prop::WRAP)),
            p("gap", T::Int, false, P(prop::GAP)),
            p("padding", T::Int, false, P(prop::PADDING)),
            p("paddingX", T::Int, false, P(prop::PADDING_X)),
            p("paddingY", T::Int, false, P(prop::PADDING_Y)),
            p("align", T::Enum(CROSS_ALIGNS), false, P(prop::CROSS_ALIGN)),
            p("justify", T::Enum(JUSTIFIES), false, P(prop::JUSTIFY)),
            p("grow", T::Int, false, P(prop::GROW)),
            p("width", T::Size(prop::WIDTH_FRACTION), false, P(prop::WIDTH)),
            p("height", T::Size(prop::HEIGHT_FRACTION), false, P(prop::HEIGHT)),
            p("maxWidth", T::Size(prop::MAX_WIDTH_FRACTION), false, P(prop::MAX_WIDTH)),
            p("maxHeight", T::Size(prop::MAX_HEIGHT_FRACTION), false, P(prop::MAX_HEIGHT)),
            p("bg", T::Enum(COLORS), false, P(prop::BG)),
            p("border", T::Enum(COLORS), false, P(prop::BORDER)),
            p("radius", T::Enum(RADII), false, P(prop::RADIUS)),
            p("position", T::Enum(POSITIONS), false, P(prop::POSITION)),
            p("top", T::Int, false, P(prop::TOP)),
            p("left", T::Int, false, P(prop::LEFT)),
            p("right", T::Int, false, P(prop::RIGHT)),
            p("bottom", T::Int, false, P(prop::BOTTOM)),
        ]
    };
}

/// The style props of a Level 2 box (`Box`, `Pressable`, `Scroll`): the
/// keys that a partial style (`hover`, `compact`, ...) can set.
pub const BOX_STYLE: &[PropSpec] = box_props![@style];

/// The style props of a `Span`.
pub const SPAN_STYLE: &[PropSpec] = &[
    p("size", T::Enum(TEXT_SIZES), false, P(prop::TEXT_SIZE)),
    p("weight", T::Enum(WEIGHTS), false, P(prop::WEIGHT)),
    p("italic", T::Bool, false, P(prop::ITALIC)),
    p("mono", T::Bool, false, P(prop::MONO)),
    p("fg", T::Enum(COLORS), false, P(prop::FG)),
    p("align", T::Enum(ALIGNS), false, P(prop::ALIGN)),
    p("lines", T::Int, false, P(prop::LINES)),
    p("grow", T::Int, false, P(prop::GROW)),
];


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
            p("disabled", T::Bool, false, P(prop::DISABLED)),
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
            p("trailing", T::Str, false, P(prop::TRAILING)),
            p("selected", T::Bool, false, P(prop::SELECTED)),
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
            p("disabled", T::Bool, false, P(prop::DISABLED)),
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
    // -- UI API 1.4 --
    ControlSpec {
        name: "Icon",
        kind: ControlKind::Icon,
        props: &[
            p("name", T::StrOneOf(ICONS), true, P(prop::ICON)),
            p("tone", T::Enum(TONES), false, P(prop::TONE)),
            // Decorative by default (hidden from AccessKit); giving a
            // `label` makes it an accessible, named icon (SPEC.md §6.3).
            p("label", T::Str, false, P(prop::LABEL)),
        ],
        children: ChildKind::None,
    },
    // -- UI API 1.5 --
    ControlSpec {
        name: "Chart",
        kind: ControlKind::Chart,
        props: &[
            p("label", T::Str, true, P(prop::LABEL)),
            p("kind", T::Enum(CHART_KINDS), true, P(prop::CHART_KIND)),
            p("data", T::ChartPoints, true, P(prop::DATA)),
            // Multi-series bar/line (SPEC.md §6.3): when given, the
            // renderer draws every series and `data` is the fallback a
            // single-series host can show.
            p("series", T::ChartSeries, false, P(prop::SERIES)),
        ],
        children: ChildKind::None,
    },
    ControlSpec {
        name: "DatePicker",
        kind: ControlKind::DatePicker,
        props: &[
            p("label", T::Str, true, P(prop::LABEL)),
            p("value", T::ValueStr, true, Target::Value),
            p("mode", T::Enum(DATE_PICKER_MODES), false, P(prop::MODE)),
            p("onChange", T::CallbackStr, false, Ev(event::CHANGE)),
            p("disabled", T::Bool, false, P(prop::DISABLED)),
        ],
        children: ChildKind::None,
    },
    // -- UI API 1.6: Level 2 styled primitives (docs/UI-ADVANCED.md) --
    ControlSpec {
        name: "Box",
        kind: ControlKind::Box,
        // `label` names the box as a group for assistive technology.
        props: box_props![
            p("label", T::Str, false, P(prop::LABEL)),
            p("onKeyDown", T::CallbackStr, false, Ev(event::KEY_DOWN)),
            p("onKeyUp", T::CallbackStr, false, Ev(event::KEY_UP)),
        ],
        children: ChildKind::Nodes,
    },
    ControlSpec {
        name: "Span",
        kind: ControlKind::Span,
        props: &[
            p("size", T::Enum(TEXT_SIZES), false, P(prop::TEXT_SIZE)),
            p("weight", T::Enum(WEIGHTS), false, P(prop::WEIGHT)),
            p("italic", T::Bool, false, P(prop::ITALIC)),
            p("mono", T::Bool, false, P(prop::MONO)),
            p("fg", T::Enum(COLORS), false, P(prop::FG)),
            p("align", T::Enum(ALIGNS), false, P(prop::ALIGN)),
            p("lines", T::Int, false, P(prop::LINES)),
            p("grow", T::Int, false, P(prop::GROW)),
            p("compact", T::PartialStyle(SPAN_STYLE), false, P(prop::COMPACT)),
            p("regular", T::PartialStyle(SPAN_STYLE), false, P(prop::REGULAR)),
            p("wide", T::PartialStyle(SPAN_STYLE), false, P(prop::WIDE)),
        ],
        children: ChildKind::Text,
    },
    ControlSpec {
        name: "Pressable",
        kind: ControlKind::Pressable,
        // A label and a role are required (docs/UI-ADVANCED.md §5).
        props: box_props![
            p("label", T::Str, true, P(prop::LABEL)),
            p("role", T::Enum(PRESSABLE_ROLES), true, P(prop::ROLE)),
            p("onPress", T::Callback0, true, Ev(event::PRESS)),
            p("disabled", T::Bool, false, P(prop::DISABLED)),
            p("onKeyDown", T::CallbackStr, false, Ev(event::KEY_DOWN)),
            p("onKeyUp", T::CallbackStr, false, Ev(event::KEY_UP)),
        ],
        children: ChildKind::Nodes,
    },
    ControlSpec {
        name: "Scroll",
        kind: ControlKind::Scroll,
        props: box_props![
            p("label", T::Str, false, P(prop::LABEL)),
            p("onKeyDown", T::CallbackStr, false, Ev(event::KEY_DOWN)),
            p("onKeyUp", T::CallbackStr, false, Ev(event::KEY_UP)),
        ],
        children: ChildKind::Nodes,
    },
];

pub fn by_name(name: &str) -> Option<&'static ControlSpec> {
    CONTROLS.iter().find(|c| c.name == name)
}

pub fn by_kind(kind: ControlKind) -> &'static ControlSpec {
    CONTROLS.iter().find(|c| c.kind == kind).expect("every control kind has a spec")
}

pub const ACCENTS: &[&str] = &["teal", "blue", "indigo", "purple", "pink", "red", "orange", "green"];
