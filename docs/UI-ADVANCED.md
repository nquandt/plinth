# Advanced UI: styled primitives (draft)

Status: draft for discussion (2026-10-06). Nothing in this document is built yet.

Today Plinth has one UI level: semantic controls (SPEC.md §6). Apps declare intent, and the runtime owns layout, spacing and color. This document adds a second level for apps that need more: **styled primitives** that map almost directly to what gpui already offers. It is not CSS. There are no selectors, no cascade and no general layout engine beyond the flexbox and grid model that gpui already uses.

## 1. Two levels

| | Level 1: semantic (simple mode) | Level 2: styled primitives (advanced mode) |
|---|---|---|
| What the app writes | `Screen`, `Section`, `Row`, `Button`, ... | `Box`, `Span`, `Pressable`, `Scroll`, `Canvas`, ... with style props |
| Who owns the look | The runtime | The app, from theme tokens |
| Layout | The runtime | Flexbox and grid (gpui's taffy model) |
| Accessibility | Automatic | Required props, checked by the compiler |
| Adapts to platforms | Fully | Through width classes and tokens |

**Islands, not whole apps.** Level 2 elements live inside Level 1 containers: a `Section`, a `Sheet`, or a new `Surface` control. The shell stays semantic: screens, navigation, toolbars, dialogs and the consent UI. So every app keeps the same navigation, back behavior and accessibility in the shell, and the advanced parts are where the app needs them (a note editor, a board, a chart, a game).

## 2. Map to gpui

The host builds gpui elements from the op stream on each frame, as it does today for semantic controls. A primitive becomes a gpui element with a style:

| Plinth primitive | gpui element |
|---|---|
| `Box` | `div()` with a `StyleRefinement` |
| `Span` (inline text with style) | `StyledText` / text runs |
| `Pressable` | `div()` with `on_click`, `hover`, `active`, `focus`, and a tab stop |
| `Scroll` | `div().overflow_y_scroll()` (or `uniform_list` / `list` when virtualized) |
| `Canvas` | `canvas()` with `PathBuilder` (SPEC.md §6.3, `ui.canvas`) |
| `Img` | `img()` from package assets |

**Style props** are a typed subset of gpui's `Styled` methods:

- layout: `direction`, `wrap`, `grow`, `shrink`, `basis`, `align`, `justify`, `gap`, `grid` (columns, rows, span), `position` (`relative` or `absolute` with insets), `overflow`;
- size: `width`, `height`, `min`/`max`, `aspect`;
- space: `padding`, `margin`, from a spacing scale (tokens), with raw pixels as an exception;
- look: `bg`, `fg`, `border` (width, color), `radius`, `shadow`, `opacity`, from **theme tokens** (`surface.raised`, `accent`, `danger`, `text.muted`, ...), not raw colors;
- text: `size` (type scale), `weight`, `italic`, `mono`, `align`, `lines` (clamp);
- states: `hover`, `active`, `focus`, `disabled`: each a partial style, as gpui's `hover(|s| ...)`;
- motion: `transition` (property, duration, easing) and `spring`, mapped to gpui's motion, spring and style transition support;
- width classes: `compact`, `regular`, `wide`: each a partial style, for adaptive layouts (SPEC.md §6.1).

Example:

```tsx
<Box direction="row" gap={2} padding={[2, 3]} bg="surface.raised" radius="md"
     hover={{ bg: "surface.hover" }} compact={{ direction: "column" }}>
  <Span weight="semibold">{title()}</Span>
  <Span fg="text.muted" lines={1}>{preview()}</Span>
</Box>
```

**Why tokens.** Colors and spacing come from the theme, so dark mode, high contrast, and the platform look still work. A raw color is a lint warning, not an error.

## 3. The web host

gpui's layout is the CSS flexbox and grid model (taffy implements it). So the web host maps the same style record to a small, fixed set of inline CSS properties on a `<div>`. It does not need a CSS parser or its own layout engine. The tokens become CSS custom properties. States become classes that the renderer generates once for each style record. Both hosts use the same tests: headless screenshots on the desktop (`plinth-shoot`) and in Edge, compared by layout boxes, not by pixels.

## 4. Wire format and size

- The compiler checks style props at compile time (types in `std/ui.d.ts`; unknown or wrong values are errors).
- A static style becomes a compact binary **style record**, interned: the guest sends each distinct record one time, and elements refer to it by id. A dynamic prop sends only the field that changed.
- Style records are a new part of the UI API, versioned like controls (append-only ids in `ui-api.toml`).

## 5. Accessibility

Level 2 can make inaccessible UI, so the compiler checks it:

- `Pressable` needs `label` (or text content) and a `role` (`button`, `link`, `checkbox`, `tab`, ...). Without it, the build fails.
- `Box` has no role by default (a layout node); `role="group"` or `"list"` with a `label` is optional.
- The host maps roles to AccessKit (desktop) and ARIA (web), as for semantic controls.
- Focus order follows the tree; `Pressable` is a tab stop.

## 6. What this is not

- Not full CSS: no selectors, no cascade, no inheritance except text style inside a `Span` tree, no floats, no pseudo-elements, no media queries (width classes stand in for them).
- Not raw gpui access: an app cannot call Rust code or reach the window. The host interprets a fixed vocabulary.
- Not a security feature: Level 2 needs no capability. The Hub can show a "custom interface" note, because such an app looks different from other apps.

## 7. Phases

1. **U1:** `Box`, `Span`, `Pressable`, `Scroll` with layout, space, tokens and text, on the desktop and the web. A gallery example. Screenshot tests on both hosts.
2. **U2:** states (`hover`, `active`, `focus`, `disabled`), width classes, transitions and springs.
3. **U3:** gestures and drag (`onDrag`, drop targets), which also give drag and drop for the notes app (docs/VALIDATION.md V2).
4. **U4:** `Canvas` in the same layer (7GUIs Circle drawer).

The notes app is a good first user: a live preview of Markdown needs styled `Span` runs inside the editor.

## 8. Open questions

- UA-Q1: Typed object props (as above), or a short class string like Tailwind (`"flex gap-2 p-3"`), which matches gpui's method names? Object props get full `tsc` checks; strings need template literal types.
- UA-Q2: Raw pixel values: allow them, or only the spacing scale plus `fraction` sizes?
- UA-Q3: Mobile: if a mobile host does not use gpui, the same vocabulary must map to native views. Keep the vocabulary small enough for that.
- UA-Q4: Can built-in semantic controls be rebuilt on Level 2 later, so that the runtime has one rendering path?
