# Kanban probe: findings

`examples/kanban` is a Jira-like issue board. It uses only the public UI API (UI API 1.13). The goal was to find where the UI model stops. This file lists what worked, what was hard, and what to change. It drives the next UI work. It uses Simplified Technical English.

The app has four columns, cards (Level 2 `Pressable`, `Box`, `Span`), drag and drop, a card menu, keys, a create and edit `Dialog` with six fields, a detail `Sheet`, a filter bar, `store.sql` with FTS5, a `Chart`, and a brand `[theme]`. About 900 lines of TSX and TS.

How it was checked:

- `plinth test examples/kanban`: 12 tests (`app/board.test.tsx`).
- `crates/plinth-shoot/tests/kanban_layout.rs`: headless desktop window with a real `store.sql` file. Layout at compact, regular and wide, in light and dark; a real mouse drag; the dialog and the sheet; 510 cards. It writes `target/shots/kanban-*.png`.
- `web/test/run-kanban.mjs`: the web host in Node (drop events, saved order).
- Not checked: a real browser (no screenshot of the web board), macOS, Linux, a screen reader, touch.

## 1. What worked

| # | Result |
|---|---|
| W1 | Cards, type badges, label pills, initials avatars, filter chips and a column switch are all made of `Box`, `Pressable` and `Span` with token names only. No raw color or pixel. The look is good in light and dark. |
| W2 | The `[theme]` table gave a brand look with no code. The contrast guard passed the first time. Charts, the dialog and the sheet follow it. |
| W3 | Width-class partial styles (`compact`, `regular`, `wide`) work for `width`, `direction` and `maxHeight`. The same code gives a switch and one column on a compact window, and four columns on a wide window. |
| W4 | `dragValue` on a `Pressable`, `onDrop` on a card, on a column `Scroll` and on a column `Box`: all work. The drop on a card gives `y`, so the app can put the card before or after the target. The real mouse drag test passes on the desktop. The web drop event moves the card in Node. |
| W5 | A column `Scroll` with fixed-height cards builds only the cards in view. With 510 cards the AccessKit tree has about 8 cards for each column. |
| W6 | A `Dialog` with six fields (`TextField`, `TextArea`, three `Picker`, `TextField`) fits a 400 by 760 window (it is 620 px high). The first field has the focus. `onSubmit` on the title field saves. |
| W7 | `store.sql` was simple: `migrate`, `batch`, `query<T>`, and an FTS5 table with three triggers (the `examples/notes` pattern). A prefix search, quotes and operators all work. The order of cards is saved in a `pos` column. |
| W8 | `Menu open={signal}` with `onContextMenu` on a card gives the keyboard and touch path of the drag. Keys on a focused card (`onKeyDown`) give a second path: `e`, `Delete`, `u`, `d`, and the arrow keys. |

## 2. What was hard or impossible

Each item has the workaround that the app uses, and a proposed change.

### Layout and width

**H1. The app cannot read the width class, and no style can hide an element.**
The compact window must show one column. The wide window must show all of them.
- Workaround: the column wrapper is a `Scroll` with `maxHeight={0}` (a `Scroll` clips). The chosen column has `maxHeight={400}`. The `regular` and `wide` partial styles set `maxHeight={400}` for all columns.
- Side effects: the folded columns are still in the AccessKit tree and may be tab stops (not tested). Their `gap` still takes space. The switch has the same trick the other way.
- Proposal: `hidden: true` in a partial style (`compact={{ hidden: true }}`), which removes the element from layout, focus and the accessibility tree. Or a reactive `widthClass()`.

**H2. A `Screen` limits the content width, so a board never uses a wide window.**
At 1280 px, the board area is about 620 px wide. Three columns of 208 px show, and the fourth is behind a sideways scroll. The first design (288 px columns) showed two and a half.
- Workaround: narrow columns (52 units = 208 px) and a sideways `Scroll`. The user must scroll to reach Done, also on a 1920 px window.
- Proposal: a `Section` or `Scroll` prop to use the full window width (`bleed`), or a `Screen` `layout="full"` for boards, editors and tables.

**H3. No auto-scroll during a drag (known, `docs/ui.md`), so a drag cannot reach a column that is out of view.**
This is real here: Done is out of view at 1280 px. The drag test could only drop on In Progress.
- Workaround: the keys (`ArrowRight` moves to the next column), the card menu ("Move to Done") and the "Move to" chips in the detail sheet.
- Proposal: auto-scroll of the nearest `Scroll` ancestor near its edge while a drag is active, in both directions.

**H4. No sticky header, no `overflow`, no min size.**
A column header cannot stay on top of its own scroll unless the header is outside the scroll. Nested scrolls were needed: a wrapper `Scroll` (for H1) around a box with the header and an inner `Scroll` for the cards. There is no `minWidth`, so a wide window cannot share the space between columns with a floor; columns have a fixed width instead.
- Proposal: `minWidth`, `minHeight`, `overflow="hidden"`, and `sticky` on a child of a `Scroll`.

**H5. A `Picker` does not shrink.**
Four segmented options overflowed a narrow row (the assignee filter at compact width) and the sheet (the four column names, "Backlog" was clipped).
- Workaround: one `Picker` for each full-width row; in the sheet, four `Pressable` chips.
- Proposal: a segmented `Picker` falls back to a menu or a wrapped group when the options do not fit.

**H6. The `Chart` is tall and cannot be sized.**
At compact width it used most of the first screen. The summary went below the board.
- Workaround: the summary is the last section.
- Proposal: `Chart size="small"` (one row), or let a `Chart` take `height` in units. A stacked bar was not tried with `Canvas`; it would give a compact header but without the hidden table of the `Chart` for assistive technology.

### Look

**H7. No shadow, no opacity, no animation, no elevation.**
Cards are separated by a 1 px border and a surface color only. A drop shows no motion. The card cannot lift when dragged. `docs/ui.md` says motion is not built.
- Proposal: `shadow="sm" | "md"` (token based), `opacity`, and `transition` on `hover`, `bg` and `maxHeight` as in `UI-ADVANCED.md` §2.

**H8. No custom fonts and only a 6-step text scale.**
The card title and the key use `size`, `weight` and `mono`. This is enough for the board. A brand font is not possible (by design; not tried).

**H9. Only the tokens of the host.**
There is no `warning` color. The priority "Medium" uses `success` (green), as the task asked. Meaning must not rest on color, so every priority also has a symbol and a word ("!! Urgent", "^ High", "= Medium", "v Low").
- Proposal: add a `warning` token, and allow `[theme]` extra tokens (for example `column-1` to `column-4`).

**H10. The icons `close` and `more` draw an empty box in the headless screenshot** (the close button of the sheet, the overflow of the toolbar). The icons are host-owned and I did not change them. This may be the font of the test machine only. Not checked in a real window.

### Language and compiler

**H11. `PL3006`: the `.map` callback gets one argument.** `COLUMNS.map((name, i) => ...)` is an error. Workaround: a helper function that takes the index, called four times.
**H12. `PL3001`: `cond ? "auto" : 0` fails** ("the branches have different types") for a `SizeValue`. Workaround: numbers only (`400` instead of `"auto"`). Proposal: type a ternary of two literals by the expected prop type.
**H13. `store.sql` parameters must have one type in a non-literal array.** The rows of `batch` need `params: string[]`, so the app writes ids and places as text (`"7"`) and relies on the SQLite column affinity. It works, but it is a trap. Proposal: allow `(string | number | boolean | null)[]` as a parameter list.
**H14. A component receives no props.** A component runs once, and a `.map` list is keyed by position. So a card is a plain function `card(issue)` that returns JSX, called from `.map`. It works, but the list of a column is keyed by index, not by issue id. A move rebuilds the props of every card after the move point (not measured). Proposal: an explicit `key` for a `.map` child (`key={i.id}`), as `List` has.
**H15. Partial style values must be literals**, so a style cannot change with a signal in `compact={{...}}`. The app puts the signal in the base prop and the literal in the partial style (H1). It is workable but it needs care.

### Interaction and accessibility

**H16. A `Pressable` has no pressed or selected state.** The filter chips and the column switch show their state with `bg`, `border` and a text mark "(on)" or "(here)". An assistive technology does not tell the state, except through the visible text.
- Proposal: `pressed?: boolean` (`aria-pressed`) and `role="tab"` with `selected`.

**H17. `onKeyDown` has no modifier keys.** Alt+Up for "move the card up" is not possible, so the app uses plain letters (`u`, `d`). A plain letter on a focused card can clash with a text-entry habit. Proposal: give `(key, modifiers)`.

**H18. One shared `Menu` with a fixed list of actions.**
The card menu shows "Move to Backlog" also for a card that is in Backlog. An `Action` has no `disabled` or `hidden`. Whether a conditional `Action` in the array works was not tried.
- Proposal: `Action disabled`, and a reactive `actions` list.

**H19. A destructive `Button` does not confirm; only a destructive `Action` does.** The detail sheet deletes at once. There is no undo. Proposal: `Button role="destructive" confirm`, or an app-level undo toast.

**H20. The drag image is the host's small label card, not the card.** (known, `docs/ui.md`). The source shows faded. Fine for a board, but a user cannot see the real card while dragging.

**H21. Tab order of folded columns** (H1) and the `Pressable` count: a column with 100 cards has 100 tab stops. A roving tabindex (one tab stop for the list, arrows inside) is not available for Level 2.
- Proposal: `role="list"` or `"grid"` on a `Box` with roving focus.

### The test kit (`plinth:test`)

**H22. The kit cannot send a context menu, key, pointer or drop event.** The tests of this app call the model functions (`dropOnCard`, `cardKey`, `moveIssue`) directly, and the shoot test sends a real mouse drag. The card menu itself has no UI test. Proposal: `q.drop(value)`, `q.key("u")`, `q.contextMenu()`.
**H23. `await` of a host promise in a test deadlocks.** A test that calls `allIssues()` and awaits it gets "the test did not finish". The test must call `settle()` first (helper `rows()` in the test). Proposal: `await` of a host promise inside a test should run the host answers by itself.
**H24. An expectation about the screen is checked at the next `await`**, not where it is written. A change between the expectation and the next `await` makes the test fail in a confusing way (found in "keeps the issues in the database"). Workaround: `await settle()` after a group of expectations. Proposal: check at once, or say so in the failure text.
**H25. The queries match the label exactly, and a `Span` with the same text as a control label matches too.** "Priority" matched both a filter `Span` and a `Picker`. Names had to be made unique ("Show priority", "Assigned to").

### Performance and size

**H26. 500 cards.** With fixed-height cards the host builds only the cards in view (W5). Without a fixed `height`, it would build all of them. The cost of a frame in a debug build at 1280 by 800, with AccessKit on: about 180 ms with 10 cards and about 280 ms with 510 cards (`kanban_layout.rs`). Every change reads the whole table again (up to 510 rows) and sets the `issues` signal; the move of a card is shown at once and written after. The time of the 500-row insert and reload was not measured separately.
The fixed card height has a price: a title is cut at two lines (`lines={2}`), and a label row cannot wrap.
- Proposal: virtualize a `Scroll` of cards with a known item height (`itemHeight`), not only children that each carry `height`.

**H27. App size.** `app.wasm` is 52 KB in a debug build (`examples/notes` is 22 KB). Every `Box`, `Span` and `Pressable` with style props costs code. The budget entry in `budgets.toml` is large for this reason. Not compared in a release build.

## 3. The next UI work, by value

1. H1 `hidden` in partial styles (or a width class signal). It removes the hack in every adaptive Level 2 app.
2. H2 full-width layout for boards, and H3 auto-scroll during a drag. Without them, a board is poor on wide windows.
3. H16 `pressed` and `selected` on `Pressable`, and H21 roving focus. These are accessibility.
4. H22 to H24 test kit steps for drop, key and context menu, and the deadlock on `await`.
5. H7 shadow and transitions; H6 a smaller `Chart`; H5 a `Picker` that fits.
6. H11 to H15 compiler friction: `.map` index, ternary of literals, mixed SQL parameters, keys for `.map`.
