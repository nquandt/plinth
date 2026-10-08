# Guide: make your app accessible

This guide tells you what the Plinth runtime does for accessibility, and what you must do yourself. It is for app authors. The reference is [ui.md](../ui.md) ("Accessibility", "Level 2: styled primitives").

## 1. Level 1 controls: what you get

Level 1 controls (`Button`, `TextField`, `Toggle`, `List`, `Row`, `Dialog` and the others in [ui.md](../ui.md)) declare intent. The runtime owns their look and their accessibility:

- **A label is required.** Each interactive control has a required `label` (or `title`). If you leave it out, the compiler gives the error `PL4003` ("`<Pressable>` needs the prop `label`", for example).
- **Roles, names and states.** On the desktop, the runtime gives each node an AccessKit role, name and state. On the web, the renderer writes semantic HTML: `<button>`, `<input>`, `<dialog>`, headings and lists.
- **Keyboard.** These are guaranteed on the desktop and the web, and the desktop is tested headless (`crates/plinth-shoot/tests/a11y_controls.rs`):
  - Tab and Shift+Tab go through each interactive control in screen order: the navigation, the toolbar, then the screen from the top. A disabled control is not a tab stop. A `Picker`, a `Tabs` and the options of a long `Picker` are one tab stop each (the chosen option).
  - Enter or Space presses a `Button`, an `Action`, a `Row` with `onPress`, a `Pressable` and a `Menu` item. Space flips a `Toggle` and a `Checkbox`.
  - Text fields take typing. Tab leaves a `TextArea`; it does not type a tab character.
  - The arrow keys choose the option before or after in a `Picker` and a `Tabs` (the focus follows), and Home and End choose the first and the last. Arrow keys, Home, End, PageUp and PageDown move a `Slider`. Up and Down step a `NumberField`. In the open `DatePicker`, the arrow keys move the day, PageUp and PageDown the month, and Enter picks the day. Up and Down move in an open `Menu`.
  - An open `Menu` or `DatePicker` has the focus; Escape closes it and gives the focus back to its trigger.
  - The menu key or Shift+F10 on the focused element runs its `onContextMenu` (UI API 1.13), with the center of the element as the position. An element with `onContextMenu` is a tab stop. A `Menu` that the app opens there has the focus, and Escape gives the focus back to the element. If you open a context menu with a right-click, also give a keyboard path: the menu key does it, and a visible button is better (`examples/7guis/circle-drawer` has both).
  - A `Sheet`, a `Dialog` and the host dialogs take the focus when they open and keep it: Tab and Shift+Tab stay in them. Escape closes them (also from a text field in them), and the focus goes back to the control that had it before.
- **Visible focus.** The control with the keyboard focus has a ring in the accent color (2 px). On the web, it is the `:focus-visible` outline.
- **Images.** `Image` needs an `alt` text that is not empty (`PL4009`).
- **Icons.** An `Icon` is decorative by default (hidden from assistive technology). Give it a `label` when it carries meaning.
- **Errors.** `TextField.error` shows a message under the field and marks the field as invalid (`aria-invalid` on the web).
- **Destructive actions.** An `Action` with `role="destructive"` asks for confirmation, unless you set `confirm={false}`.
- **Light and dark.** The theme follows the system setting (the window appearance on the desktop, `prefers-color-scheme` on the web).

## 2. Level 2 primitives: what you must do

`Box`, `Span`, `Pressable`, `Scroll` and `Canvas` give you your own look. Then you also own more of the accessibility.

| Primitive | Accessibility | What you must do |
|---|---|---|
| `Pressable` | A button or a link. A tab stop. Enter (and Space for a button) presses it. | `label` and `role` (`"button"` or `"link"`) are required. Write a label that tells what happens: "Reset the score", not "Reset". |
| `Box` | No role. With `label`, a named group. | Give a `label` to a box that groups related content, and to each box with `onKeyDown` or `onKeyUp`: a key handler makes the box a tab stop. |
| `Scroll` | A scroll view. A tab stop on the web. | Give a `label` when the content needs a name. |
| `Span` | A text label. | Do not give meaning by color only. Add a word or a symbol. |
| `Canvas` | A drawing. `label` is required. | Describe what the drawing shows, and update the label when the data changes. |

More rules:

- **Empty labels.** The compiler checks that a required label is present. It does not check that the text is useful: `label=""` compiles. Write real words.
- **Visible focus.** The runtime draws its focus ring on each element that takes the focus (a `Pressable`, a box with a key handler). A `focus` style, for example `focus={{ border: "accent" }}` (as `examples/pong` does), adds to the ring.
- **Keyboard and pointer.** Each action that you give to a pointer (`onPointerDown`, a drag) must also have a keyboard way: a `Pressable`, or a key handler. `examples/pong` moves the paddle with a drag, with the arrow keys, and with buttons.
- **Colors.** Use theme tokens only (`"text"`, `"surface"`, `"accent"`, ...). There are no raw colors, so your app follows the light and dark theme.

## 3. Example

This example is checked with `plinth check` against `std/ui.d.ts`.

```tsx
import { app, signal, Screen, Section, Box, Span, Pressable, Canvas, rect, Button, TextField } from "plinth:ui";
import { Math } from "plinth:core";

const count = signal(0);
const name = signal("");

function Home() {
  return (
    <Screen title="Cards">
      <Section title="Level 1">
        <TextField label="Your name" value={name} error={name().trim() === "" ? "Type a name" : ""} />
        <Button label="Add one" role="primary" onPress={() => count.set(count() + 1)} />
      </Section>
      <Section title="Level 2">
        <Box label="Score card" direction="row" gap={3} padding={3} radius="md" bg="surface" border="border">
          <Span weight="semibold" grow={1}>Score</Span>
          <Span>{`${count()}`}</Span>
        </Box>
        <Pressable label="Reset the score" role="button" padding={2} radius="md" border="border"
                   focus={{ border: "accent" }} onPress={() => count.set(0)}>
          <Span>Reset</Span>
        </Pressable>
        <Canvas label={`A bar of length ${count()}`} viewWidth={100} viewHeight={10}
                shapes={[rect(0, 0, Math.min(count() * 10, 100), 10, "accent")]} />
      </Section>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Cards", component: Home } } });
```

## 4. Test your app

### 4.1 The web axe run

`web/test/a11y.html` runs your app in the web host and checks the page with axe-core. You need a clone of the Plinth repository, Node 22, and network access (the page loads axe-core from `cdnjs.cloudflare.com`).

1. Build your app and export the core:

   ```sh
   plinth build <your app folder>
   plinth core export target/core.wasm
   ```

2. Serve the repository root with a static server, for example `npx http-server . -p 8080`. The `.plnt` file must be under the served folder.
3. Open `http://localhost:8080/web/test/a11y.html?app=<URL of your .plnt>&core=<URL of core.wasm>` in a browser.
4. When the check ends, the page shows the result as JSON: a list of `violations`, each with `id`, `impact`, `help` and the nodes.

Fix each violation with the impact `serious` or `critical`. `node web/test/run-a11y.mjs` does the same check for the apps in its `APPS` list, in headless Microsoft Edge (it uses the Edge path in the script), and fails on a serious or critical violation. `A11Y_VERBOSE=1` prints the nodes of each violation.

### 4.2 By hand

The automatic check does not find all problems. Also do these checks:

1. Use the app with the keyboard only: Tab, Shift+Tab, Enter, Space, the arrow keys, Escape. Each action must be possible, and you must always see where the focus is.
2. Use a screen reader on the desktop (for example Narrator on Windows) and in a browser. Each control must have a name that tells its purpose.
3. Use the app in the light and the dark theme.

## 5. Limits

These are not done yet (`docs/LIMITS.md`, `docs/PRODUCTION.md` U3–U7):

- The app does not follow the system settings for high contrast, text scale and reduced motion.
- IME input (for example Chinese or Japanese) is not tested on the desktop.
- There is no test that compares the web HTML with the desktop AccessKit tree, and the desktop tree is not checked with a screen reader yet (Narrator, VoiceOver, Orca).
- On the desktop, a text field has its name on the group around it, not on the text input node itself (a gpui-ce limit). A screen reader can read the field without its label.
- The roles differ on some controls between the hosts: see the table in `docs/PRODUCTION.md` U3.
