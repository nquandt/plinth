# UI control reference (UI API 1.13)

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
| `onCleanup(f)` | `(f: () => void) => void` | Core 1.12. Runs `f` when the current component, or the part of the UI that a condition or a list row made, goes away. Stop a timer or a frame loop there. |
| `isActive()` | `() => boolean` | Core 1.12. True while the app's window is in the foreground (desktop: the window is active; web: the page is visible and has the focus). Reactive: an `effect` that reads it runs again on a change. `examples/pong` pauses there. |
| `onMount(f)` | `(f: () => void) => void` | Core 1.12 (`docs/GAPS.md` G10). Runs `f` once, after the commit of the event or `init` that put the current component's elements in the host tree, not during rendering. A signal write inside `f` is a normal update: the runtime flushes again and sends the result in the same reply. A component inside a condition or a list row runs its own `onMount` again each time it appears, paired with its own `onCleanup`. |
| `onUpgrade(handler)` | `(handler: (fromVersion: string) => void \| Promise<void>) => void` | Core 1.13 (`PRODUCTION.md` D4). Call it at the top level. `handler` runs once at start when the host has a stored data version that is lower than the app's version; `fromVersion` is that stored version. A first install, the same version and a downgrade run no handler. The host stores the new version when the handler returns (or its promise resolves). A throw or a rejection is an uncaught error, and the handler runs again at the next start. `docs/migrations/core-1.13.md` §2 has an example. |

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
| `Dialog` | `open: Signal<boolean> \| boolean`, `title`, `message?`, `actions: Element[]`, `onClose?`, `children?` | An alert or confirmation built from your UI, as opposed to the host-owned `plinth:dialog` module (see [host-apis.md](host-apis.md)). UI API 1.13: the children are a small form (a `Slider`, a `TextField`) between the message and the actions. See "Dialog with a form". |
| `Menu` | `label`, `actions: Element[]`, `open?: Signal<boolean> \| boolean`, `onClose?` | An anchored popover menu. With `open` (UI API 1.13), the app opens it: it has no button, and it shows at the point of a context-menu event. See "Context menus". |

### Dialog with a form (UI API 1.13)

```tsx
<Dialog open={adjusting} title="Adjust diameter" message="The circle at (200, 150)." onClose={closeAdjust}
        actions={[<Action label="Done" role="primary" onPress={closeAdjust} />]}>
  <Slider label="Diameter" value={diameter} min={4} max={100} step={1} />
</Dialog>
```

- The children show between the message and the actions. Keep the form
  small: one to three fields. For a longer form, use a `Sheet`.
- When the dialog opens, its first field has the focus. Tab and
  Shift+Tab stay in the dialog.
- Enter in a field does not close the dialog. To act on Enter in a
  `TextField`, give it `onSubmit`.
- Escape and a click outside the dialog close it: `open` becomes false
  and `onClose` runs. An action does not close the dialog: set `open` to
  false in its `onPress`.
- A host with an older UI API shows no children. `examples/7guis/circle-drawer`
  uses a dialog with a slider.

## Collections

| Control | Props | Notes |
|---|---|---|
| `List<T>` | **items: T[]**, **key: (item: T) => string \| number**, **row: (item: T) => Element**, `empty?`, `onReorder?: (from, to) => void` | Virtualized, keyed. `empty` shows when `items` is empty. With `onReorder` (UI API 1.13), the user can move the rows: see "Drag and drop". |
| `Row` | **title**, `subtitle?`, `icon?: IconName`, `onPress?`, `trailing?: string`, `selected?`, `onContextMenu?: (x, y) => void`, `dragValue?: string`, `onDrop?: (value, x, y) => void`, `children?` | The standard list row. `onContextMenu` and `onDrop` (UI API 1.13) get the position in spacing units. |
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
- Drawing (desktop): `bar` draws grouped bars; `line`
  draws one stroked polyline per series over "nice" y-axis ticks and
  gridlines, with point markers when the points are not too dense and
  x labels thinned by width class; `pie` draws filled wedges with a
  legend of each label and its share (beside the pie, or below it at
  compact width). Colors always come from the theme's chart palette.
  The web host draws the same kinds as inline SVG.
- Values (both hosts): `bar` and `line` use one scale that always
  includes zero. A negative value goes below the zero line (a bar grows
  down from it); a zero value has no bar. `pie` uses the magnitude of
  each value; a zero value has no wedge and no legend entry, and a share
  that rounds to 0 % shows as "<1%".
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
  jumps. The same is true for a plain `value` plus `onChange`: while the
  `change` event runs, the runtime does not send back the value that the
  host shows already (two fields that set each other do not loop; see
  `examples/7guis/temperature`).
- `TextField.error` shows a message under the field and marks it as
  invalid (red border; `aria-invalid` on the web). An empty string means
  no error, so `error={bad() ? "Not a number" : ""}` works. The same is
  true for an empty `Row.trailing`.
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

## Level 2: styled primitives (UI API 1.6)

For the parts of an app that need their own look (a card, a board, an
editor), four primitives take typed style props (docs/UI-ADVANCED.md).
They go inside the semantic shell: a `Screen`, a `Section` or a `Sheet`.

| Primitive | What it is | Accessibility |
|---|---|---|
| `Box` | A flex box. | No role; with `label`, a named group. |
| `Span` | Styled text (the children). | A text label. |
| `Pressable` | A box that the user presses. `label`, `role` (`"button"` or `"link"`) and `onPress` are required. | A button or a link; a tab stop; Enter (and Space for a button) presses it. |
| `Scroll` | A box that scrolls along its `direction`. Give it a `height` or `maxHeight` for a column. | A scroll view; a tab stop on the web. |

Box props (also on `Pressable` and `Scroll`): `direction` (`"column"`,
the default, or `"row"`), `wrap`, `gap`, `padding`, `paddingX`, `paddingY`,
`align` (`"stretch"`, `"start"`, `"center"`, `"end"`), `justify`
(`"start"`, `"center"`, `"end"`, `"between"`), `grow`, `width`, `height`,
`maxWidth`, `maxHeight`, `bg`, `border`, `radius` (`"none"`, `"sm"`,
`"md"`, `"lg"`, `"full"`). Span props: `size` (`"xs"` … `"2xl"`), `weight`,
`italic`, `mono`, `fg`, `align`, `lines` (the most lines), `grow`.

- **Spaces and sizes** are spacing units: one unit is 4 px on every host.
  A size can also be a fraction string: `"auto"`, `"full"`, `"1/2"`,
  `"1/3"`, `"2/3"`, `"1/4"`, `"3/4"` (a string literal). An element with a
  width or a height in units does not shrink. Sizes (`width`, `height`,
  `maxWidth`, `maxHeight`) and insets can be fractional units (UI API
  1.11): `left={10.25}` is 41 px. Use them for motion; `gap` and the
  paddings take whole units.
- **Colors** are theme tokens: `"background"`, `"surface"`, `"surface.alt"`,
  `"accent"`, `"danger"`, `"success"`, `"text"`, `"text.muted"`,
  `"on.accent"`, `"border"`, `"hover"`, `"selected"`, `"none"`. There are no
  raw colors and no raw pixels.
- **`.map()` children** (and a `List`) inside a primitive take part in its
  layout: mapped cards in a row `Box` are a row.
- **Long scrolls.** On the desktop, a column `Scroll` with 40 or more
  children that each have a fixed `height` builds only the children near
  its viewport; a row with 16 or more fixed-`width` children inside a
  horizontal `Scroll` builds only the ones in view. Give the rows of a long
  table a `height` and its cells a `width` (`examples/7guis/cells`).
- **Positioning (UI API 1.9).** `position="absolute"` takes a box out of
  the flow; `top`, `left`, `right` and `bottom` (spacing units) place it in
  its parent primitive. Later children are drawn over earlier ones.
- **Keys (UI API 1.9).** `onKeyDown` and `onKeyUp` on `Box`, `Pressable`
  and `Scroll` get the key name: `"ArrowUp"`, `"ArrowDown"`, `"ArrowLeft"`,
  `"ArrowRight"`, `"Enter"`, `"Escape"`, `"Space"`, `"Tab"`,
  `"Backspace"`, `"Delete"`, `"Home"`, `"End"`, `"PageUp"`, `"PageDown"`,
  or a lower-case letter or digit. A held key gives one down and one up
  (no repeats). An element with a key handler is a tab stop: give it a
  `label`. `examples/pong` uses both.
- **Pointer (UI API 1.12, core 1.12).** `onPointerDown`, `onPointerMove`
  and `onPointerUp` on `Box`, `Pressable`, `Scroll` and `Canvas` get
  `(x, y)`: the position from the top-left corner of the element, in
  spacing units on a box and in view units on a `Canvas` (fractional). A
  mouse, a pen or a finger. After a pointer-down in the element, it also
  gets the moves outside it and the pointer-up, so a drag works. On the
  web, a box with `onPointerMove` takes touch moves itself (no page scroll).
  `examples/pong` aims the paddle with a drag on the court. A pointer
  event does not go to an element under a `Sheet`, a `Dialog` or an open
  menu panel.
- **Context menu (UI API 1.13).** `onContextMenu` on `Box`, `Pressable`,
  `Scroll`, `Canvas` and `Row` gets `(x, y)` in the same units as the
  pointer events. See "Context menus".
- **Drag and drop (UI API 1.13).** `dragValue` on `Box`, `Pressable` and
  `Row`; `onDrop(value, x, y)` on `Box`, `Pressable`, `Scroll`, `Canvas`
  and `Row`. See "Drag and drop".
- **Hide an element (UI API 1.14).** `hidden` is a style prop on `Box`,
  `Pressable`, `Scroll` and `Span`. It is also a key of a partial style:
  `compact={{ hidden: true }}`, `regular`, `wide`, `hover`, `active`,
  `focus`. A hidden element is not in the layout (it leaves no gap), it is
  not a tab stop, and it is not in the accessibility tree. The desktop
  host does not build its children. On the web it is `display: none`. A
  width class changes the state at once when the window changes. In a
  partial style, `hidden: false` shows an element that the base props
  hide.
- **Full width (UI API 1.14).** A `Screen` keeps its content in a column
  of a readable width. `<Screen layout="full">` uses the whole window
  width, for a board, a table or an editor. `"readable"` is the default.
- **Least size (UI API 1.14).** `minWidth` and `minHeight` on `Box`,
  `Pressable` and `Scroll` take spacing units or a fraction, as `width`
  does. The element does not get smaller than this size. Use `grow` with
  `minWidth` to let columns share a row and keep a floor.
- **Pressed and selected (UI API 1.14).** `pressed` (a boolean) on a
  `Pressable` with `role="button"` makes it a toggle button, such as a
  filter chip. Assistive technology tells "pressed" or "not pressed"
  (web: `aria-pressed`; desktop: the AccessKit toggled state). The
  compiler gives error `PL4010` when `pressed` is on a link, or when the
  role is not the literal `"button"`. `selected` (a boolean) marks the
  current item of a group, such as the column switch or a list of
  choices. Assistive technology tells "selected" (web: `aria-selected`,
  or `aria-current` on a link; desktop: the AccessKit selected state).
  A pressed or selected `Pressable` without its own `bg` is drawn with
  the `selected` token as its background. Do not use color as the only
  mark: the text or a border can also change. Not built yet: a roving
  focus for a list of `Pressable` (one tab stop, arrows inside; finding H21
  in `docs/KANBAN-FINDINGS.md`).
- The runtime gives a `Pressable` its hover and disabled states.
- **Partial styles (UI API 1.7).** `hover`, `active` and `focus` (on `Box`,
  `Pressable`, `Scroll`) and `compact`, `regular`, `wide` (also on `Span`)
  take an object of style props, applied over the element's props in that
  state or width class: `hover={{ border: "accent" }}`,
  `compact={{ direction: "column" }}`. The values must be literals; the
  compiler encodes the object at compile time. An app `hover` replaces the
  runtime's hover of a `Pressable`. Motion (transitions, springs) is not
  built yet.

```tsx
<Pressable label="Open the inbox" role="button" direction="row" gap={3} padding={3}
           radius="md" bg="surface" border="border" onPress={open}>
  <Span weight="semibold" grow={1}>Inbox</Span>
  <Span fg="text.muted">3 new</Span>
</Pressable>
```

`examples/primitives` shows all four, and a Canvas.

### Canvas (UI API 1.10)

```tsx
import { Canvas, rect, circle, line, canvasText } from "plinth:ui";

<Canvas label="A sun over the ground" viewWidth={200} viewHeight={100} maxWidth={120}
        shapes={[rect(0, 80, 200, 20, "success"), circle(170, 25, 12, "accent"),
                 line(0, 80, 200, 80, "border", 2), canvasText(8, 96, "Hello", "text.muted", 9)]} />
```

- The shapes use a view space of `viewWidth` × `viewHeight`. The canvas
  keeps that aspect ratio and scales it to its width (by default the full
  width; `width`, `maxWidth` and `grow` as for a box). Text scales too.
- `rect(x, y, width, height, color)`, `circle(cx, cy, r, color)`,
  `line(x1, y1, x2, y2, color, width?)`, `canvasText(x, y, text, color,
  size?)`, and the outlines `strokeRect(x, y, width, height, color,
  lineWidth?)` and `strokeCircle(cx, cy, r, color, lineWidth?)` (UI API
  1.13; the line is centered on the edge, as an SVG stroke). Colors are
  theme tokens. `shapes` is any `Shape[]`: a literal,
  a variable, a `computed` or a `.map()` result; a signal read in it makes
  the drawing update.
- `label` is required: the canvas is one image for assistive technology.
- Pointer events (UI API 1.12) and `onContextMenu` (UI API 1.13) give
  positions in view units: see Level 2 "Pointer". A canvas with
  `onContextMenu` is a tab stop. `examples/7guis/circle-drawer` uses them.

### Context menus (UI API 1.13)

```tsx
const menu = signal(false);

<Canvas label="Drawing" viewWidth={400} viewHeight={300} shapes={shapes()}
        onContextMenu={(x, y) => { select(x, y); menu.set(true); }} />
<Menu label="Circle" open={menu}
      actions={[<Action label="Adjust diameter..." onPress={adjust} />]} />
```

- `onContextMenu(x, y)` runs for a secondary (right) click, for a long
  press on a touch screen (web: when the browser sends `contextmenu`; the
  browsers on Android do, Safari on iOS does not), and for the menu key or
  Shift+F10 while the element has the focus. For a key, `x` and `y` are the
  center of the element. An element with `onContextMenu` is a tab stop.
- Only the innermost element with a handler gets the event. On the web,
  the browser's own menu shows only where the app has no handler.
- A `Menu` with `open` has no button. Set `open` to true in the
  `onContextMenu` handler: the menu then shows at the point of the event
  (for a key, the center of the element). A `Menu` that the app opens at
  another time shows where it is in the layout.
- The open menu has the focus on its first action. Up and Down move,
  Enter and Space press, Escape closes it. Escape, a click outside it, or an
  action closes it: `open` becomes false (a signal is set) and `onClose`
  runs. Then the focus goes back to where it was.
- A click outside the menu also goes to the element under the pointer, as
  in a browser. If that is not what you want, check in the handler that the
  menu is closed (`examples/7guis/circle-drawer` does).
- A host that does not know UI API 1.13 does not send the event and shows
  the `Menu` as a button.
- Not built yet: paths and curves, images.

### Drag and drop (UI API 1.13)

Drag and drop works inside one app. It does not work between apps, or
with the files and the text of the operating system (`docs/GAPS.md` G4).

```tsx
// The user moves the tasks of a list.
<List items={tasks()} key={(t) => t.id} onReorder={(from, to) => moveTask(from, to)}
      row={(t) => <Row title={t.title} />} />

// The user drags a card onto a column.
<Box label="Done" direction="column" gap={2} onDrop={(id, x, y) => setColumn(id, "done")}>
  <Box label="Write the guide" dragValue="card-7" padding={2} bg="surface" radius="md" />
</Box>
```

- **`onReorder(from, to)` on `List`.** The user moved the row at index
  `from` of `items` to index `to`. Move the item in your data: the host
  does not change the order itself. The user can:
  - drag a row onto another row. A line shows where the row goes: under
    the target row when the row moves down, over it when it moves up.
  - press Alt+Up or Alt+Down while the row, or a control in it, has the
    focus. A `Row` in such a list is a tab stop. A row of another kind
    needs a control that takes the focus.
- **`dragValue` on `Box`, `Pressable` and `Row`.** The user can drag the
  element. The value is a string that you choose (a key). A row of a list
  with `onReorder` can have a `dragValue` too: a drop on another row moves
  it, and a drop on a target gives the value.
- **`onDrop(value, x, y)` on `Box`, `Pressable`, `Scroll`, `Canvas` and
  `Row`.** An element with `dragValue` was dropped on this element. `x`
  and `y` are in the units of the pointer events. While a drag is over
  the element, the host shows it as a target (the selected color and an
  accent border). An element does not take its own drop. Each target
  takes each value: ignore the values that you do not want. Only the
  innermost target gets the drop. On a row with `onDrop`, a drop of a row
  of the same list is a drop, not a move.
- **How a drag starts.** With a mouse or a pen: a press of the main
  button and a move. With a finger (web): a long press (400 ms) and then
  a move; a move before that scrolls the page. A long press with no move
  gives the context menu when the finger goes up (Android). A press in a
  text field or on a slider is not a drag. Escape stops a drag on the web.
- **What the user sees.** A small card with the label of the element
  (`title`, `label` or the value) moves with the pointer, and the source
  shows faded.
- **Modal layers.** A drag does not start on an element under a `Sheet`,
  a `Dialog` or an open menu, and nothing under them takes a drop.
- **Keyboard.** Alt+Up and Alt+Down move a row. For a drop target, give
  the source `onContextMenu` and a `Menu` with "Move to ..." actions (see
  "Context menus"), or a button.
- **Assistive technology.** A polite live region tells "Dragging Apple",
  "Moved Apple to position 3 of 3" and "Dropped Apple". A row of a list
  with `onReorder` tells its keys (`aria-keyshortcuts` on the web, the
  AccessKit keyboard shortcut on the desktop).
- **Hosts.** The desktop uses the gpui-ce drag and drop. The web uses
  pointer events, not the HTML drag-and-drop API: that API does not work
  with a finger and shows the browser's drag image. It works in the
  sandboxed app frame. A host that does not know UI API 1.13 shows no drag
  and sends no event: keep another path (buttons) if the move is
  necessary there.
- Not built yet: drag and drop between apps and with the operating
  system, a drag card that the app draws, auto-scroll of a `Scroll` near
  its edge during a drag, reorder in a `Grid`. `examples/todo` moves its
  tasks.

## Theme and system settings

- The runtime owns the theme: the type scale, the color tokens, the
  corner radii and the focus ring. An app chooses its `accent` and can
  set the values of the color tokens in a `[theme]` table (see "Theming an
  app" below). It never uses raw colors in its code.
- The theme follows the system settings, at start and when they change
  (PRODUCTION U7):
  - **Dark mode:** the window appearance on the desktop,
    `prefers-color-scheme` on the web.
  - **High contrast:** a high-contrast variant of the tokens. Text and
    its background are at least 7:1, `border` is the text color,
    `text.muted` is not faint, and the focus ring is 3 px. On Windows,
    the OS high-contrast colors win. On the web, `prefers-contrast:
    more`; in `forced-colors: active` the browser colors win.
  - **Text size:** the Windows "Text size" setting (100 % to 225 %) or
    the browser default font size scales all text. Buttons, the
    navigation and rows grow with it. On the desktop, the space inside
    the standard controls grows too (it is in rem); the space of a `Box`
    (spacing units) does not.
  - **Reduced motion:** no host transitions, and no caret blink in a
    text field.
- Colors in a `Box`, a `Span` and a `Canvas` are tokens, so they follow
  the settings too. An app cannot read the settings: an animation with
  `onFrame` keeps its motion.
- For tests: `PLINTH_THEME=dark|light`, `PLINTH_HIGH_CONTRAST=1|0`,
  `PLINTH_TEXT_SCALE=2` (or `200%`) and `PLINTH_REDUCED_MOTION=1|0` force
  a setting on the desktop (`plinth run`, `plinth shoot`).

### Theming an app

An app can change the VALUES of the color tokens. It does not change the
token NAMES: the app code still writes `bg="surface"` or `fg="accent"`, and
there are no raw colors in the code. Each app has its own values; the
runtime owns everything else (the type scale, the spacing, the corner radii
and the focus ring).

Write a `[theme]` table in `plinth.toml`:

```toml
[theme]
border = "#e2cdb2"                              # one value for light and dark
background = { light = "#fff8ee", dark = "#1b1410" }
text = { light = "#2b1a0c", dark = "#f7eadb" }

[theme.accent]                                  # the same as an inline table
light = "#b4440a"
dark = "#ff9a4d"
```

- **Keys.** `background`, `surface`, `surface-alt`, `border`, `hover`,
  `selected`, `text`, `text-muted`, `accent`, `on-accent`, `danger`,
  `success`. A key that is not in this list is a build error. A table has at
  most 12 keys.
- **Values.** A hex string `"#rrggbb"` sets light and dark. A table
  `{ light = "#rrggbb", dark = "#rrggbb" }` sets each. Nothing else is
  allowed: no alpha, no names, no `rgb(...)`.
- **What stays computed.** The host derives the hover tint of the accent,
  the text selection color, the toggle track and knob, and the backdrop of
  a dialog from the base tokens. `selected` follows the accent unless you
  set it. A token that you do not set keeps the host value.
- **Accent.** The named `accent` of `app({ accent })` still works. When
  `[theme]` has an `accent`, it wins.
- **No permission.** The table is data in the manifest, so it is in the
  signed package. It needs no capability and shows no prompt.

**The contrast guard.** `plinth build` and `plinth validate` merge your
values with the host values for light and for dark. Then they check ALL of
these pairs on the merged result. A pair is checked even when you do not set
its tokens, so the host values that fill the other tokens must work with
yours.

| Pair | Least ratio |
|---|---|
| `text` on `background` | 4.5:1 |
| `text` on `surface` | 4.5:1 |
| `on-accent` on `accent` | 4.5:1 |
| `text` on `selected`, over `surface` | 4.5:1 |
| `text` on `selected`, over `background` | 4.5:1 |
| `text-muted` on `background` | 3:1 |
| `danger` on `background` | 3:1 |
| `success` on `background` | 3:1 |
| toggle track on `surface` | 1.5:1 |
| toggle knob on the track | 1.5:1 |

- **`selected`.** The desktop paints `selected` with alpha over the row
  below it. When you set `selected`, it is one opaque color. When you do not
  set it, it is the accent at 12 % (light) or 16 % (dark). The guard uses
  the color over `surface` and over `background`.
- **Toggle.** The track and the knob are fixed host colors: you cannot set
  them, and they are not derived from your tokens. The guard only stops a
  theme from hiding the track on its `surface`. The ratio is 1.5:1, not the
  3:1 of WCAG 1.4.11 for non-text parts, because the host track is a quiet
  grey (1.6:1 on the default light `surface`) and 3:1 would reject the host
  itself. The state of the toggle is also shown by the position of the knob
  and the accent color.
- **Host accents.** Each named accent has a light value with at least 4.5:1
  against white `on-accent`, and a dark value with at least 4.5:1 against
  the dark `on-accent`. The light teal (the default) and the light orange
  are a little darker than in earlier versions for this reason. This
  changes the look of every app that uses them.

The error names the pair, the ratio and the least ratio, for example:
``[theme] light mode: `text` on `background` has a contrast of 1.17:1; the
least is 4.5:1``. The host values fill the tokens that you do not set, so an
override of `background` alone passes only when the host `text` is readable
on it in light AND in dark. For a light background, the host dark text is
faint, so set `text`, `text-muted`, `danger` and `success` with it, or give
`background` a value for each appearance (`examples/themed`).

**At load time.** A host checks the table again (a package is never
trusted). A table that fails any check is ignored, with one log line, and
the host theme is used. The app never stops because of its theme.

**What the override does not change:**

- **High contrast** (Windows high contrast, `prefers-contrast: more`) and
  web **forced colors**: the override is ignored. The host high-contrast
  tokens win, so the 7:1 rule and the OS colors hold.
- **Host-owned parts** keep the host theme: the consent dialog, the Hub
  window around an app, the "This app stopped" banner and the error banner,
  the update prompt and the dialogs of the host. Only the content of the
  app window takes the override. On the web, the Hub page around the app
  frame is a different document, so it is not affected.
- **Charts** take their colors from the merged `accent` and `text-muted`.

Both hosts give the same colors for the same manifest: the web host fills
the tokens that the app does not set from the same host palette that the
desktop uses. A web page of an app with no `[theme]` is unchanged. Tested
on Windows desktop and in Node and Chromium only. macOS, Linux and iOS are
not tested.

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
- Interactive controls take keyboard focus (Tab and Shift+Tab, in
  screen order) and activation (Enter or Space), and show a focus ring.
  Arrow keys choose in a `Picker` and a `Tabs` and move a `Slider`, a
  `NumberField` and the open `DatePicker`. A `Sheet` or a `Dialog` keeps
  the focus while it is open and closes on Escape.
  `docs/guides/accessibility.md` lists all the keys.
- `Menu` and the toolbar overflow use AccessKit roles `menu` and
  `menu item`; they close on an outside click, Escape, or after an
  action runs. The menu key and Shift+F10 run `onContextMenu` on the
  focused element (UI API 1.13).
- Alt+Up and Alt+Down move the focused row of a `List` with `onReorder`
  (UI API 1.13). A polite live region tells the result of a drag or a
  move.
- A `Sheet` or a `Dialog` blocks the pointer to the controls under it,
  and a drag under it does not start.

## Versioning

The control set is the **UI API**, versioned `MAJOR.MINOR`
independently of the framework's own version. A minor version only adds
controls or optional props; a major version can remove or change them.
Your package declares the UI API version it was built against
(`ui-api` in the package manifest, which `plinth build` writes); a host supports a range. An unknown
prop is a compile error; a host that sees a control kind newer than it
supports renders a placeholder and logs an error — it does not crash.

This page documents **UI API 1.14**, the version in `std/ui.d.ts` today. UI API 1.14 adds `hidden` (also in partial styles), `minWidth`, `minHeight`, `Screen` `layout`, and `pressed` and `selected` on `Pressable`; it adds no control and removes nothing. A heading that names a version tells when that control or prop came in.
