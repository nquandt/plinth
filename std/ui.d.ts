// plinth:ui: the app entry, reactivity, navigation and the controls of
// UI API 1.0 (SPEC.md §6 and §7). Every control requires a label or a title,
// so the runtime can make it accessible.

// -- Reactivity ---------------------------------------------------------------

/** A reactive value. Call it to read; `set` and `update` write. */
export interface Signal<T> {
  (): T;
  set(value: T): void;
  update(f: (value: T) => T): void;
}

/** A derived reactive value. Call it to read. */
export interface Computed<T> {
  (): T;
}

export declare function signal<T>(initial: T): Signal<T>;
export declare function computed<T>(f: () => T): Computed<T>;
export declare function effect(f: () => void): void;

// -- App ----------------------------------------------------------------------

export type Accent = "teal" | "blue" | "indigo" | "purple" | "pink" | "red" | "orange" | "green";
export type IconName =
  | "house" | "gear" | "check" | "list" | "plus" | "trash" | "star" | "info" | "number"
  | "search" | "edit" | "close" | "back" | "forward" | "calendar" | "clock" | "user" | "mail" | "heart"
  | "bell" | "share" | "download" | "upload" | "refresh" | "filter" | "menu" | "more" | "lock" | "warning";

export interface ScreenDef {
  title: string;
  icon?: IconName;
  component: () => JSX.Element;
}

export interface AppConfig {
  accent?: Accent;
  screens: { [name: string]: ScreenDef };
  /** The top-level destinations, in order. The default is all screens. */
  primary?: string[];
}

export interface App {
  readonly __plinthApp: true;
}

/** The app entry. `export default app({...})` in `app/main.tsx`. */
export declare function app(config: AppConfig): App;

/** Stack navigation (SPEC.md §6.2, UI API 1.2). Call it directly to select a
 * primary screen; `.push`/`.back` work the stack inside the current tab. */
export declare const navigate: {
  /** Selects a primary screen by its name in `screens`. */
  (screen: string): void;
  /** Pushes a screen onto the current primary tab's stack. */
  push(screen: string): void;
  /** Pops the current primary tab's stack. A no-op at the bottom. */
  back(): void;
};

// -- Controls -----------------------------------------------------------------

type Element = JSX.Element;
type Child = Element | false | null;
type Children = Child | Child[];
type TextContent = string | number | (string | number)[];

export type Tone = "default" | "muted" | "danger" | "success";

export declare function Screen(props: { title: string; actions?: Element[]; children?: Children }): Element;
export declare function Section(props: { title?: string; footer?: string; children?: Children }): Element;

export type Align = "start" | "center" | "end";

/** Lays out its children along an axis. In a row, each child gets the same
 * width. "auto" is a row on wide windows and a column on narrow ones. */
export declare function Group(props: { axis?: "auto" | "row" | "column"; children?: Children }): Element;

export declare function Text(props: {
  style?: "body" | "caption" | "mono";
  tone?: Tone;
  align?: Align;
  children?: TextContent;
}): Element;
export declare function Heading(props: { level?: 1 | 2 | 3; align?: Align; children?: TextContent }): Element;

export declare function Button(props: {
  label: string;
  role?: "primary" | "default" | "destructive";
  /** "large" makes a taller key, for keypads and main actions. */
  size?: "default" | "large";
  onPress: () => void;
  disabled?: boolean;
}): Element;

export declare function TextField(props: {
  label: string;
  /** A signal binds both ways. A string needs `onChange`. */
  value: Signal<string> | string;
  placeholder?: string;
  error?: string;
  onChange?: (value: string) => void;
  onSubmit?: () => void;
  disabled?: boolean;
}): Element;

export declare function Toggle(props: {
  label: string;
  /** A signal binds both ways. A boolean needs `onChange`. */
  value: Signal<boolean> | boolean;
  onChange?: (value: boolean) => void;
  disabled?: boolean;
}): Element;

export declare function List<T>(props: {
  items: T[];
  key: (item: T) => string | number;
  row: (item: T) => Element;
  /** Shown when `items` is empty. */
  empty?: Element;
}): Element;

export declare function Row(props: {
  title: string;
  subtitle?: string;
  icon?: IconName;
  onPress?: () => void;
  /** A muted value shown on the right of the row, e.g. "12 items" or "$4.50". */
  trailing?: string;
  children?: Children;
  /** This row is the selected one of its list (shown, and told to assistive technology). */
  selected?: boolean;
}): Element;

export declare function Empty(props: { title: string; message?: string }): Element;

// -- UI API 1.2 inputs --

export declare function Checkbox(props: {
  label: string;
  /** A signal binds both ways. A boolean needs `onChange`. */
  value: Signal<boolean> | boolean;
  onChange?: (value: boolean) => void;
  disabled?: boolean;
}): Element;

export declare function TextArea(props: {
  label: string;
  /** A signal binds both ways. A string needs `onChange`. */
  value: Signal<string> | string;
  placeholder?: string;
  onChange?: (value: string) => void;
  disabled?: boolean;
}): Element;

export declare function Slider(props: {
  label: string;
  /** A signal binds both ways. A number needs `onChange`. */
  value: Signal<number> | number;
  min: number;
  max: number;
  step?: number;
  onChange?: (value: number) => void;
  disabled?: boolean;
}): Element;

export declare function NumberField(props: {
  label: string;
  /** A signal binds both ways. A number needs `onChange`. */
  value: Signal<number> | number;
  min?: number;
  max?: number;
  step?: number;
  onChange?: (value: number) => void;
  disabled?: boolean;
}): Element;

export declare function Picker(props: {
  label: string;
  /** A signal binds both ways. A string needs `onChange`. */
  value: Signal<string> | string;
  /** <= 4 options render as a segmented control; more render as a list. */
  options: string[];
  onChange?: (value: string) => void;
  disabled?: boolean;
}): Element;

export declare function Progress(props: {
  label?: string;
  /** 0 to 1. Missing means indeterminate. */
  value?: number;
}): Element;

export declare function Badge(props: {
  label: string;
  tone?: Tone;
}): Element;

// -- UI API 1.2: structure ------------------------------------------------

/** A value, not shown by itself: pass it inside `actions={[...]}` on
 * `Screen`, `Dialog` or `Menu`. */
export declare function Action(props: {
  label: string;
  onPress: () => void;
  icon?: IconName;
  role?: "primary" | "default" | "destructive";
  /** Destructive actions ask for confirmation unless this is `false`. */
  confirm?: boolean;
}): Element;

/** In-screen segmented tabs. */
export declare function Tabs(props: {
  items: string[];
  value: Signal<string> | string;
  onChange?: (value: string) => void;
}): Element;

/** A modal: a bottom sheet on a narrow window, a side panel or dialog on a
 * wide one. */
export declare function Sheet(props: {
  open: Signal<boolean> | boolean;
  title: string;
  onClose?: () => void;
  children?: Children;
}): Element;

/** An alert or confirmation. */
export declare function Dialog(props: {
  open: Signal<boolean> | boolean;
  title: string;
  message?: string;
  actions: Element[];
}): Element;

export declare function Menu(props: { label: string; actions: Element[] }): Element;

/** Like `List`, with a responsive column count instead of one row per item. */
export declare function Grid<T>(props: {
  items: T[];
  key: (item: T) => string | number;
  cell: (item: T) => Element;
  empty?: Element;
}): Element;

// -- UI API 1.3 -----------------------------------------------------------

export type Aspect = "square" | "wide" | "tall";

/** An image from the project's `assets/` (SPEC.md §6.3, §10.1). `src` must
 * be a string literal naming a file under `assets/`. */
export declare function Image(props: {
  /** A file name under `assets/`, for example "logo.png". */
  src: string;
  /** Required: a non-empty description, for people using a screen reader. */
  alt: string;
  aspect?: Aspect;
}): Element;

// -- UI API 1.4 -----------------------------------------------------------

/** An icon from the runtime icon set (SPEC.md §6.3). Decorative by default
 * (hidden from screen readers); pass `label` to give it an accessible name. */
export declare function Icon(props: { name: IconName; tone?: Tone; label?: string }): Element;

/** A date, time or date-and-time field with a popover picker (SPEC.md §6.3).
 * `value` holds ISO text: "YYYY-MM-DD" (date), "HH:MM" (time) or
 * "YYYY-MM-DDTHH:MM" (datetime); an empty string means no value. A signal
 * binds both ways. A string needs `onChange`. */
export declare function DatePicker(props: {
  label: string;
  value: Signal<string> | string;
  mode?: "date" | "time" | "datetime";
  onChange?: (value: string) => void;
  disabled?: boolean;
}): Element;

// -- UI API 1.5 -----------------------------------------------------------

export interface ChartPoint {
  label: string;
  value: number;
}

export interface ChartSeriesDef {
  name: string;
  points: ChartPoint[];
}

/** A data-driven chart (SPEC.md §6.3). The runtime chooses the colors (from
 * the theme's chart palette), the height, axis ticks, value labels, and a
 * legend when one is needed; apps never set pixels or colors. `data` and
 * each `points` array is an array of `{ label, value }` objects: a literal,
 * a `ChartPoint[]` variable, a signal or computed, or a `.map()` result. A
 * signal read in it makes the chart update like any other reactive prop. `series` draws several series over `data`
 * (bar and line only), and can also be a literal, a `ChartSeriesDef[]` variable, a computed or a `.map()` result; `data` stays the fallback a host without `series`
 * support can show. */
export declare function Chart(props: {
  label: string;
  kind: "bar" | "line" | "pie";
  data: ChartPoint[];
  series?: ChartSeriesDef[];
}): Element;

// -- Level 2: styled primitives (UI API 1.6, docs/UI-ADVANCED.md) ----------
// Layout, space and look from typed props and theme tokens. Spaces and
// sizes are spacing units (one unit is 4 px on every host); a size can also
// be a fraction of the parent. There are no raw pixels and no raw colors.

type ColorToken =
  | "none"
  | "background"
  | "surface"
  | "surface.alt"
  | "accent"
  | "danger"
  | "success"
  | "text"
  | "text.muted"
  | "on.accent"
  | "border"
  | "hover"
  | "selected";
/** A size in spacing units (fractional units are allowed, UI API 1.11), or a share of the parent box. */
type SizeValue = number | "auto" | "full" | "1/2" | "1/3" | "2/3" | "1/4" | "3/4";

/** The style props of a box: also the keys of a partial style (`hover`, `compact`, ...). Partial style values must be literals. */
interface BoxLook {
  /** The direction of the children. Default "column". */
  direction?: "row" | "column";
  wrap?: boolean;
  /** Space between the children, in spacing units. */
  gap?: number;
  padding?: number;
  paddingX?: number;
  paddingY?: number;
  /** The children across the direction. Default "stretch". */
  align?: "stretch" | "start" | "center" | "end";
  /** The children along the direction. Default "start". */
  justify?: "start" | "center" | "end" | "between";
  /** The share of the free space that this element takes in its parent box. */
  grow?: number;
  width?: SizeValue;
  height?: SizeValue;
  maxWidth?: SizeValue;
  maxHeight?: SizeValue;
  bg?: ColorToken;
  border?: ColorToken;
  radius?: "none" | "sm" | "md" | "lg" | "full";
  /** "absolute": the element leaves the flow and the insets place it in its parent box (UI API 1.9). */
  position?: "relative" | "absolute";
  /** Insets in spacing units, from the edges of the parent box. Fractional units are allowed (UI API 1.11): `left={10.25}`. */
  top?: number;
  left?: number;
  right?: number;
  bottom?: number;
}

interface BoxStyle extends BoxLook {
  /** Applied while the pointer is over the element. */
  hover?: BoxLook;
  /** Applied while the element is pressed. */
  active?: BoxLook;
  /** Applied while the element has keyboard focus. */
  focus?: BoxLook;
  /** Applied in the compact width class (narrower than 600 px). */
  compact?: BoxLook;
  /** Applied in the regular width class (600 to 1199 px). */
  regular?: BoxLook;
  /** Applied in the wide width class (1200 px or more). */
  wide?: BoxLook;
  children?: Children;
}

/**
 * Keys for `onKeyDown`/`onKeyUp` (UI API 1.9): "ArrowUp", "ArrowDown",
 * "ArrowLeft", "ArrowRight", "Enter", "Escape", "Space", "Tab",
 * "Backspace", "Delete", "Home", "End", "PageUp", "PageDown", a lower-case
 * letter or a digit. A held key gives one down and one up (no repeats).
 * An element with a key handler is a tab stop; give it a `label`.
 */
interface KeyHandlers {
  onKeyDown?: (key: string) => void;
  onKeyUp?: (key: string) => void;
}

/** A layout box. `label` names it as a group for assistive technology. */
export declare function Box(props: BoxStyle & KeyHandlers & { label?: string }): Element;

interface SpanLook {
  size?: "xs" | "sm" | "md" | "lg" | "xl" | "2xl";
  weight?: "regular" | "medium" | "semibold" | "bold";
  italic?: boolean;
  mono?: boolean;
  fg?: ColorToken;
  align?: Align;
  lines?: number;
  grow?: number;
}

/** Styled text. The children are the text. */
export declare function Span(props: {
  compact?: SpanLook;
  regular?: SpanLook;
  wide?: SpanLook;
  size?: "xs" | "sm" | "md" | "lg" | "xl" | "2xl";
  weight?: "regular" | "medium" | "semibold" | "bold";
  italic?: boolean;
  mono?: boolean;
  fg?: ColorToken;
  align?: Align;
  /** The most lines to show (0: no limit). */
  lines?: number;
  grow?: number;
  children?: TextContent;
}): Element;

/** A box that the user can press: a button or a link. It is a tab stop; Enter and Space press it. */
export declare function Pressable(
  props: BoxStyle & KeyHandlers & { label: string; role: "button" | "link"; onPress: () => void; disabled?: boolean },
): Element;

/** A box that scrolls along its direction. Give it a height or a max height. */
export declare function Scroll(props: BoxStyle & KeyHandlers & { label?: string }): Element;

// -- Canvas (UI API 1.10, docs/UI-ADVANCED.md U4) --------------------------
// Shapes in a view space of `viewWidth` x `viewHeight`. The canvas keeps
// that aspect ratio and scales the space to its width (by default the full
// width of its parent). Colors are theme tokens.

/** One shape of a canvas: make it with `rect`, `circle`, `line` or `canvasText`. */
export type Shape = string & { readonly __plinthShape: true };

/** A filled rectangle. */
export declare function rect(x: number, y: number, width: number, height: number, color: ColorToken): Shape;
/** A filled circle. */
export declare function circle(cx: number, cy: number, r: number, color: ColorToken): Shape;
/** A straight line; `width` in view units (default 1). */
export declare function line(x1: number, y1: number, x2: number, y2: number, color: ColorToken, width?: number): Shape;
/** Text with its left baseline at (x, y); `size` in view units (default 12). */
export declare function canvasText(x: number, y: number, text: string, color: ColorToken, size?: number): Shape;

/** A drawing. `label` describes it for assistive technology (required). */
export declare function Canvas(props: {
  label: string;
  viewWidth: number;
  viewHeight: number;
  shapes: Shape[];
  width?: SizeValue;
  maxWidth?: SizeValue;
  grow?: number;
}): Element;
