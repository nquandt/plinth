# UI control reference (UI API 1.5)

Every control in this page is a typed JSX intrinsic exported from
`plinth:ui`. Props in **bold** are required. This reference is checked
against `std/ui.d.ts`, the file the compiler and your editor both read,
so the names and props here are exact.

## The one rule: intent, not appearance

Your app declares controls, structure, roles, and data. It cannot set
position, size, margin, padding, color, font, or border. The runtime
owns layout and chooses it from the semantic tree and the window's
width class. This is enforced at the type level: there is no `style`
prop, no `className`, and no pixel or color value anywhere in
`plinth:ui`.

A control's intent props describe *what it means*, not *how it looks*:
`role="primary"` on a `Button`, not a blue background; `tone="danger"`
on a `Text`, not a red color; `size="large"` for a numeric keypad key,
not a height in points.

## Reactivity primitives

| Export | Signature | Notes |
|---|---|---|
| `signal<T>(initial)` | `(initial: T) => Signal<T>` | `Signal<T>` is callable (`sig()` reads) and has `.set(value)`/`.update(f)`. |
| `computed<T>(f)` | `(f: () => T) => Computed<T>` | Read-only, derived, callable. |
| `effect(f)` | `(f: () => void) => void` | Reruns when a signal it read changes. |

## App entry and navigation

```ts
export declare function app(config: AppConfig): App;
```

```ts
interface AppConfig {
  accent?: Accent;                         // "teal" | "blue" | "indigo" | "purple" | "pink" | "red" | "orange" | "green"
  screens: { [name: string]: ScreenDef };
  primary?: string[];                       // top-level destinations; default is every screen
}
interface ScreenDef {
  title: string;
  icon?: IconName;
  component: () => JSX.Element;
}
```

`navigate` is both callable and an object of methods:

| Call | Effect |
|---|---|
| `navigate("name")` | Selects a primary screen by its name in `screens`. |
| `navigate.push("name")` | Pushes a declared screen onto the current primary tab's stack. |
| `navigate.back()` | Pops the current tab's stack. A no-op at the bottom. |

The compiler checks every screen name against `app({ screens })`, so a
typo is a compile error. `push` does not take props yet. A screen that
is not listed in `primary` is reachable only through `push`.

## Icons

`IconName` is one of: `house`, `gear`, `check`, `list`, `plus`, `trash`,
`star`, `info`, `number`, `search`, `edit`, `close`, `back`, `forward`,
`calendar`, `clock`, `user`, `mail`, `heart`, `bell`, `share`,
`download`, `upload`, `refresh`, `filter`, `menu`, `more`, `lock`,
`warning`.

## Structure

| Control | Props | Notes |
|---|---|---|
| `Screen` | **title**, `actions?: Element[]`, `children?` | The root of a screen. Pass an array of `Action` elements to `actions` for the toolbar/overflow menu. |
| `Section` | `title?`, `footer?`, `children?` | A titled group. The runtime chooses a card or inset-group style. |
| `Group` | `axis?: "auto" \| "row" \| "column"`, `children?` | A logical grouping. In a row, every child gets equal width. `"auto"` behaves as a row on wider windows and a column on narrow ones. |
| `Tabs` | `items: string[]`, `value: Signal<string> \| string`, `onChange?` | In-screen segmented tabs. |
| `Sheet` | `open: Signal<boolean> \| boolean`, `title`, `onClose?`, `children?` | Modal: a bottom sheet on a narrow window, a side panel or dialog on a wide one. |
| `Dialog` | `open: Signal<boolean> \| boolean`, `title`, `message?`, `actions: Element[]` | An alert or confirmation built from your UI, as opposed to the host-owned `plinth:dialog` module (see [host-apis.md](host-apis.md)). |
| `Menu` | `label`, `actions: Element[]` | An anchored popover menu. |

## Collections

| Control | Props | Notes |
|---|---|---|
| `List<T>` | **items: T[]**, **key: (item: T) => string \| number**, **row: (item: T) => Element**, `empty?` | Virtualized, keyed. `empty` shows when `items` is empty. |
| `Row` | **title**, `subtitle?`, `icon?: IconName`, `onPress?`, `trailing?: string`, `children?` | The standard list row. |
| `Grid<T>` | **items: T[]**, **key**, **cell: (item: T) => Element**, `empty?` | Like `List`, with a responsive column count instead of one row per item (2, 3, or 4 columns, by width class). |

## Content

| Control | Props |
|---|---|
| `Text` | `style?: "body" \| "caption" \| "mono"`, `tone?: Tone`, `align?: Align`, `children?: string \| number \| (string \| number)[]` |
| `Heading` | `level?: 1 \| 2 \| 3`, `align?: Align`, `children?` |
| `Badge` | **label**, `tone?: Tone` |
| `Progress` | `label?`, `value?: number` — 0 to 1; missing means indeterminate |
| `Empty` | **title**, `message?` |
| `Image` | **src: string**, **alt: string**, `aspect?: "square" \| "wide" \| "tall"` |
| `Icon` | **name: IconName**, `tone?: Tone`, `label?: string` |

`Tone` is `"default" | "muted" | "danger" | "success"`. `Align` is
`"start" | "center" | "end"`.

`Icon` renders a glyph from the runtime icon set. It is decorative by
default (hidden from AccessKit, `aria-hidden`); pass `label` to give it
an accessible name and an AccessKit image role. `Row.icon` renders the
same glyph inline and stays the simpler way to put an icon on a row.

`Row.trailing` shows a muted value on the right of the row, after the
title and subtitle and before any children, e.g. `trailing="12 items"`
or `trailing="$4.50"`. A `Row`'s children render as trailing controls,
after `trailing`: for example a `Toggle` or a `Badge` placed inside a
`Row` appears on the row's trailing edge.

`Image.src` must be a string literal naming a file under the project's
`assets/` directory (otherwise `PL4008`); `alt` must not be empty
(otherwise `PL4009`). `plinth build` copies the referenced files into
the package under `assets/`. If the asset is missing at run time, the
host shows a placeholder with the `alt` text.

On compact windows an `Image` fills the content width at its aspect
ratio. On regular and wide windows the host caps the image height
(360 px for `square`/`tall`, 280 px for `wide`) and centers it
horizontally, keeping the aspect ratio.

## Chart (UI API 1.5)

```ts
interface ChartPoint { label: string; value: number }
interface ChartSeriesDef { name: string; points: ChartPoint[] }

declare function Chart(props: {
  label: string;
  kind: "bar" | "line" | "pie";
  data: ChartPoint[];
  series?: ChartSeriesDef[];
}): Element;
```

A data-driven chart. As with every control, the app states intent only:
`label`, `kind` and the data. The runtime (not the app) chooses the
colors (from the theme's chart palette, derived from the app's accent
and neutral tokens, distinguishable in light and dark), the height (by
width class), axis ticks, value labels, and a legend when one is
needed. There are no size, axis or color props.

- **`label`** is required (SPEC.md §6.1 item 3: every control needs a
  label for accessibility) and is the chart's accessible name.
- **`data`** is an array of `{ label, value }` objects. It can be an
  array literal (`data={[{ label: "Jan", value: total() }, ...]}`), a
  `ChartPoint[]` variable, a signal or computed read, a function call, or
  a `.map()` result (`data={items().map((i) => ({ label: i.name, value:
  i.total }))}`). The number of points can change at run time. Any object
  type with a `label: string` field and a `value: number` field is
  accepted; other fields are ignored. `ChartPoint` and `ChartSeriesDef`
  can be imported from `plinth:ui` as types. The compiler encodes the
  points into one wire string. A signal read in the expression makes the
  chart update like any other reactive prop.
- **`series`** is optional: an array of `{ name, points }` objects (an
  array literal of object literals, or any expression of an array of
  structs with a `name: string` and a `points` field, such as a
  `ChartSeriesDef[]` variable, a `computed` or a `.map()` result;
  `points` is any expression that `data` accepts), for
  overlaying several series on a `bar` or `line` chart (for example
  "this year" vs "last year"). When present, the renderer draws every
  series instead of `data`; `data` stays the single-series fallback a
  host without `series` support can show. `pie` always uses `data`
  (one series).
- Empty data (no points, or every series empty) shows a "No data"
  placeholder instead of an empty drawing.
- Drawing (desktop): `bar` draws grouped proportional bars; `line`
  draws one stroked polyline per series over "nice" y-axis ticks and
  gridlines, with point markers when the points are not too dense and
  x labels thinned by width class; `pie` draws filled wedges with a
  legend of each label and its share (beside the pie, or below it at
  compact width). Colors always come from the theme's chart palette.
- Accessibility: the desktop renderer gives the chart the AccessKit
  role `Figure`, named by `label`, plus a hidden text summary with
  every `"label: value"` pair, so a screen reader gets the numbers
  without relying on the drawing. The web renderer uses an inline SVG
  with `<title>`/`<desc>` plus a visually hidden `<table>` of the same
  data.

## Inputs

Each input binds a `value`: pass a `Signal` for a two-way binding, or a
plain value plus `onChange` for one-way control.

| Control | Props |
|---|---|
| `Button` | **label**, `role?: "primary" \| "default" \| "destructive"`, `size?: "default" \| "large"`, **onPress: () => void**, `disabled?` |
| `TextField` | **label**, **value: Signal\<string\> \| string**, `placeholder?`, `error?`, `onChange?: (value: string) => void`, `onSubmit?: () => void` |
| `TextArea` | **label**, **value: Signal\<string\> \| string**, `placeholder?`, `onChange?` |
| `NumberField` | **label**, **value: Signal\<number\> \| number**, `min?`, `max?`, `step?`, `onChange?`, `disabled?` |
| `Toggle` | **label**, **value: Signal\<boolean\> \| boolean**, `onChange?`, `disabled?` |
| `Checkbox` | **label**, **value: Signal\<boolean\> \| boolean**, `onChange?`, `disabled?` |
| `Picker` | **label**, **value: Signal\<string\> \| string**, **options: string[]**, `onChange?`, `disabled?` |
| `Slider` | **label**, **value: Signal\<number\> \| number**, **min**, **max**, `step?`, `onChange?`, `disabled?` |
| `DatePicker` | **label**, **value: Signal\<string\> \| string**, `mode?: "date" \| "time" \| "datetime"`, `onChange?` |

Notes:
- A `Signal` prop binds both ways: the host updates the control at
  once, sends a UI event, and the guest updates the signal — but the
  guest does not echo a redundant `set-prop` back, so the cursor never
  jumps.
- `Picker.options` (and `Tabs.items`) must be array literals of string
  literals; the compiler joins them at compile time. Four options or
  fewer render as a segmented control; more render as a list.
- `Slider` follows pointer drag on its track, snapped to `step`.
- `DatePicker.value` holds ISO text: `"YYYY-MM-DD"` for `mode="date"`
  (the default), `"HH:MM"` for `mode="time"`, and
  `"YYYY-MM-DDTHH:MM"` for `mode="datetime"`; an empty string means no
  value. The desktop renderer shows the value in a readable English
  form and opens an anchored popover: a month grid for `date`/
  `datetime` (previous/next month, today marked, the selected day
  highlighted, click or Enter to pick, arrow keys to move the focused
  day, PageUp/PageDown to change month, Escape to close) and
  hour/minute steppers for `time`/`datetime`. The web renderer uses a
  native `<input type="date">`, `"time"`, or `"datetime-local">`.

## Actions

| Control | Props | Notes |
|---|---|---|
| `Action` | **label**, **onPress**, `icon?: IconName`, `role?: "primary" \| "default" \| "destructive"`, `confirm?` | A value, not rendered by itself: pass an array of `Action` elements to `actions` on `Screen`, `Dialog`, or `Menu`. A destructive action asks for confirmation unless `confirm={false}`. |

The toolbar shows two actions on a narrow window and four on a medium or
wide one; the rest collapse into an overflow menu.

## Layout rules the runtime owns

- **Forms** stack fields vertically. Labels sit above the field on a
  narrow window, and beside it on a medium or wide one.
- **Spacing** comes only from runtime tokens; apps cannot set it.
- **Width:** content has a maximum readable width on wide windows, and
  is centered.
- **Sections** flow vertically in a screen; on a wide window, the
  runtime can place independent sections in two columns.
- **Button order** follows each platform's convention (primary last on
  desktop, first on mobile) — you do not control it.
- **Destructive actions** always confirm through a standard dialog,
  unless the app sets `confirm={false}` on the `Action`.

## Navigation shell by width class

| Width class | Shell |
|---|---|
| `compact` (< 600px) | Bottom tab bar for `primary` screens, with a stack navigator inside each tab. |
| `regular` (600–1200px) | Navigation rail and a stack. |
| `wide` (≥ 1200px) | Sidebar and content. |

## Accessibility

- Every interactive control requires a label (`label`, `title`, or a
  text child), which the compiler enforces at the type level — most
  controls simply have no variant without one.
- The runtime maps every node to an AccessKit role, name, and state.
- Interactive controls take keyboard focus (Tab) and activation (Enter
  or Space; arrow keys move a `Slider`).
- `Menu` and the toolbar overflow use AccessKit roles `menu` and
  `menu item`; they close on an outside click, Escape, or after an
  action runs.

## Versioning

The control set is the **UI API**, versioned `MAJOR.MINOR`
independently of the framework's own version. A minor version only adds
controls or optional props; a major version can remove or change them.
Your package declares the UI API version it was built against
(`ui-api = "1.5"` in `plinth.toml`); a host supports a range. An unknown
prop is a compile error; a host that sees a control kind newer than it
supports renders a placeholder and logs an error — it does not crash.

This page documents **UI API 1.5**, the version in `std/ui.d.ts` today.
