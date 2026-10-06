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

/** Selects a primary screen by its name in `screens`. */
export declare function navigate(screen: string): void;

// -- Controls -----------------------------------------------------------------

type Element = JSX.Element;
type Child = Element | false | null;
type Children = Child | Child[];
type TextContent = string | number | (string | number)[];

export type Tone = "default" | "muted" | "danger" | "success";

export declare function Screen(props: { title: string; children?: Children }): Element;
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
