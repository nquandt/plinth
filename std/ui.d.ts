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
