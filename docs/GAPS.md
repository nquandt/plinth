# Dogfooding gaps

Found while building `examples/budget` and `examples/utility` (two realistic
apps: a budget tracker and a desktop unit/text-tools utility). Each row is
something an ordinary TypeScript developer would reasonably try that the
compiler rejected, or a runtime/UI limitation, with the workaround used in
the app. None of these were fixed in the compiler (all need more than a
small local change), per the dogfooding task's scope.

| # | Area | What failed | Minimal repro | Error text | Severity | Suggested fix |
|---|------|-------------|----------------|------------|----------|----------------|
| 1 | Checker | `Picker.options` cannot be a variable or expression, only an array literal, so option lists that depend on other state (unit kind, category set built elsewhere) cannot be passed directly. | `const opts = ["a","b"]; <Picker label="x" value={v} options={opts} />` | `PL3001: \`options\` must be an array literal of strings` | High | Allow any `string[]`-typed expression (a local `const` array, or at least an array built from a literal via `.map`/spread) once the checker can prove it is a plain list of string literals, or document+keep the literal-only rule but allow `string[]` variables when they are declared `const` with a literal initializer. Workaround used: one `Picker` per app "kind", shown conditionally (`examples/utility/app/converter.tsx`). |
| 2 | Checker | An array produced with `.map()` cannot be used as JSX children; there is no way to render a variable number of elements except through `List`/`Grid`, even for small, non-interactive repeats (e.g. one `Progress` bar per category). | `{categories.map((c) => <Progress label={c} value={1} />)}` | `PL4004: an array of elements is not a child` (plus `PL4002: <Progress> has no prop key` for the `key` attempt) | Medium | Document the `List`/`Grid`-only rule for repeated children, or add a lightweight non-interactive repeat primitive. Workaround used: wrap `Progress` rows in `<List items={categories} key={(c) => c} row={(c) => <Progress .../>} />` (`examples/budget/app/stats.tsx`). |
| 3 | Checker/Parser | JSX fragments (`<>...</>`) parse but are rejected by the checker, so a conditional block that renders more than one sibling element needs a wrapper control. | `return <>{a}{b}</>;` inside a conditional branch | `PL4004: fragments are not supported` (help: use a `<Section>` or `<Group>`) | Low | Either stop parsing `<>` (clearer, earlier error) or support it as sugar for `<Group>`. Workaround used: `<Group axis="column">...</Group>` (`examples/utility/app/texttools.tsx`). |
| 4 | Stdlib (`lib.d.ts`) | `String` has no `split`. Any tokenizing (word splitting for a title-case transform, CSV-like parsing) must be done with a manual `slice`/character-scan loop. | `text.split(" ")` | TypeScript error: `Property 'split' does not exist on type 'string'` | Medium | Add `split(separator: string): string[]` to `std/lib.d.ts` and the checker's built-in `String` signature; implement in `plinth-rt` string routines. Workaround used: hand-written `toTitleCase` char scan (`examples/utility/app/texttools.tsx`). |
| 5 | Stdlib (`plinth:time`) | **Fixed in this pass.** No `Date` type and no calendar/timestamp formatting (no way to turn an epoch-ms number into a year/month/day/hour string). A "timestamp converter" can only do elapsed-time breakdown by hand. | Need: format `now()` as a human date | n/a (no API exists; nothing to call) | Medium | Added `timezoneOffset`, `dateParts`, `makeDate`, `formatDate`, `toISOString`, `parseDate` to `plinth:time` (see below and `docs/host-apis.md`). Workaround removed from `examples/utility`; the new "Timestamps" tab uses the real API. |
| 6 | Checker | `as` casts are rejected outright (`PL2006`), including narrow, locally-safe casts such as turning a `Picker`'s validated `string` signal value back into a small string-literal union (`"Groceries" \| "Rent" \| ...`). There is no other way to narrow a dynamic string to a literal union. | `const c: Category = draftCategory() as Category;` | `PL2006: \`as\` casts are not allowed` (help: annotate the variable type instead) | Medium | Either add a safe narrowing helper (e.g. a user-defined type guard function, which already works for `null` narrowing in the examples) that is documented as the sanctioned pattern for this case, or allow `as` to a union of string literals when the source is `string` and the checker can verify exhaustiveness isn't required. Workaround used: `Category` is a plain `string` alias instead of a literal union in `examples/budget/app/types.ts` (loses compile-time exhaustiveness checking on category names). |
| 7 | UI / rendering | On the "wide" window shot, the sidebar icon for `icon="edit"` (used on the utility app's "Text tools" primary screen) renders as a blank glyph box instead of a recognizable icon, while other icons (`#`/`number`) render correctly. | `plinth-shoot examples/utility/dist/utility.plnt out` → `screen1-regular.png`/`screen1-wide.png` sidebar | n/a (visual only, no error) | Low | Check the icon font/mapping for `edit` (and audit the rest of `IconName` the same way) in `plinth-ui`'s icon table. |

| 8 | Test harness / protocol | An enum prop (e.g. `Text`'s `style`) is stored as `Value::Enum(u16)`, not `Value::Str`, so `Node::str_prop` (used throughout `tests/e2e.rs` and `tests/apps.rs`) silently returns `None` for it; there is no `enum_prop` helper that resolves the index back to its name. This is a test-authoring trap more than an app bug: a test that filters nodes by an enum prop's string value compiles and always finds zero matches. | `h.one(ControlKind::Text, \|n\| n.str_prop(prop::STYLE) == Some("mono"))` | No compiler error; the assertion fails at runtime with "expected one Text, found 0" | Low | Add a `Node::enum_prop(prop, kind) -> Option<&str>` convenience (resolving through the kind's enum table) for test harnesses, or document the `Value::Enum` trap next to `str_prop`. Workaround used: matched on rendered text content instead (`crates/plinth-compiler/tests/apps.rs`). |

## Larger items (not started, out of scope here)

- `async`/`await` and `try`/`catch`: done (see "Fixed in this pass").
  The `Promise` API, async methods and `await` in all statements: done
  (see "Fixed in this pass"). `Promise.race`, `Promise.any` and
  `Promise.allSettled`: done (see "`Promise.race`, `any` and
  `allSettled`" in "Fixed in this pass").
- Regular expressions: not needed by either app; the text-tools screen used
  manual character scans instead of `RegExp` (which `lib.d.ts` declares as
  an empty, unusable interface).
- `Map`/`Set` iteration as plain methods (`keys()`/`values()`/`forEach`,
  fixed in gap batch 3 item 2; see "Fixed in this pass"). `entries()` as a
  plain array: done (tuples, see "Fixed in this pass").
- `Array` methods beyond the common set: done. `concat`/`reduce` (gap
  batch 3 item 3), `sort`, and `splice`/`fill`/`flat` are in "Fixed in this
  pass".

## Fixed in this pass

- #7 (fixed): `icon="edit"` rendered as a blank glyph box because U+270E
  (LOWER RIGHT PENCIL) is missing from the default font. Changed
  `icon_glyph` in `crates/plinth-ui/src/theme.rs` to U+270D (WRITING HAND),
  which renders. Verified with `plinth-shoot` on `examples/utility`
  before/after the change.
- #1 (fixed): `Picker.options`/`Tabs.items` (`PropTy::StrList`) now accept
  any `string[]` expression, not just an array literal. `check/jsx.rs`
  keeps the literal fast path (compile-time join, and the U+001F
  diagnostic, which still only applies to literals since a dynamic value
  cannot be checked at compile time); a non-literal expression is type
  checked as `string[]` and lowered to a runtime `arr_join(expr, "\u001f")`
  call, which becomes an ordinary reactive `string` prop (the same
  effect-wrapping machinery as any other prop that reads a signal in JSX).
  No new runtime function needed (`arr_join` already existed). Tests: a
  frontend test (`slider_number_picker_progress_badge` in
  `tests/frontend.rs`) exercising a `const` array variable, and an e2e
  test (`picker_options_from_a_signal_update_at_run_time` in
  `tests/lang.rs`) where a `computed`'s array drives `options` and updates
  after a button press. `examples/utility/app/converter.tsx` was simplified
  from one Picker per unit kind to a single From/To pair whose `options`
  comes from `unitsFor(kind())`.
- #4 (fixed, plus neighbors): added `String.split(sep)`, `replace(a, b)`
  (first match only), `replaceAll(a, b)`, `padStart`/`padEnd`, `charAt`,
  `lastIndexOf` to `std/lib.d.ts` and the checker (`check/expr.rs`'s
  `string_method`). `split("")` splits per Unicode scalar value (not a
  lone UTF-16 surrogate), which agrees with the UTF-16-unit `length`/
  `slice` semantics for all non-astral text; documented on `strings::split`
  in `plinth-rt/src/strings.rs`. New runtime functions (`str_split`,
  `str_replace`, `str_replace_all`, `str_pad_start`, `str_pad_end`,
  `str_last_index_of`) were added rather than compiler-generated loops,
  since `split`'s result needs GC array allocation and the others are
  simple one-shot string scans better done once in Rust; `charAt` instead
  reuses `str_slice` from the checker (no new runtime function).
  `CORE_MINOR`/`CORE_VERSION` bumped to 1.4. Tests: a new "Strings" section
  in `crates/plinth-compiler/tests/lang.rs` (includes a non-ASCII
  `split("")` case and a non-ASCII `charAt` case).
- #3 (fixed): JSX fragments (`<>...</>`) are now allowed directly in a
  JSX child position. `parse.rs`'s `jsx()` now builds the child list
  through a new `jsx_children_into` helper that recurses into
  `JSXChild::Fragment` and splices its children straight into the
  parent's list; no node is created and no new AST/IR concept is
  needed. This flattening happens at parse time, so it only applies to
  a fragment that is itself a direct JSX child; a fragment nested
  inside a dynamic expression (a ternary branch, a `.map()` callback,
  …) is not flattened (that would need a child slot that holds more
  than one element at run time, which is out of scope here) and still
  gets the diagnostic, as does a fragment used as a component's return
  value or any other standalone expression (no caller-side parent to
  splice into). Documented in `docs/language.md` ("Supported syntax").
  Tests: `fragment_as_jsx_child_splices_its_children`,
  `fragment_as_component_return_value_is_rejected`,
  `fragment_inside_a_ternary_branch_is_still_rejected` in
  `crates/plinth-compiler/tests/lang.rs`. No example app used a
  fragment workaround precisely matching this shape, so no app changes
  were needed for this item.
- #6 (fixed): `expr as T` is now allowed for two safe, checked
  narrowings, each compiled into its own run-time check that `throw`s
  (traps) on a mismatch, since Plinth has no `any` escape hatch to
  fall back on the way TypeScript's compile-time-only `as` can: (1)
  `string` (or a wider literal union) to a narrower string literal
  union, reusing the same literal-membership check already built for
  `JSON.parse<T>()` (`json_decode_str_lits` in `check/expr.rs`; a
  `Type::StrLits` is a plain run-time `string`, so the check is a
  chain of `===` comparisons, no interned-id decoding needed); and (2)
  a discriminated union to one of its members, checked by comparing
  the union's existing discriminant tag (`TExprKind::UnionTag`, the
  same mechanism `.tag` property reads use) against the target
  member's literal. Both reuse the existing `throw`/`trap` path
  (`TStmt::Throw` → the `__plinth_rt_throw` ABI function, already in
  core 1.0), so **no new runtime function was needed** and
  `CORE_MINOR` did not change. `number as int` is explicitly rejected
  with a PL2006 pointing at `int(x)`, since there is nothing to check
  that would make it safe. Casts between unrelated types still get
  PL2006. Implementation: a new `ExprKind::As` (parsed in `parse.rs`,
  previously `as` was rejected at parse time before the checker ever
  saw it) and `Checker::as_cast`/`cast_str_to_lits`/
  `cast_union_to_member` in `check/expr.rs`. Documented in
  `docs/language.md` ("Supported syntax"). Tests:
  `string_as_literal_union_narrows_and_runs`,
  `string_as_literal_union_traps_on_a_bad_value`,
  `union_as_member_narrows_and_runs`, `union_as_wrong_member_traps`,
  `number_as_int_is_rejected`, `as_cast_between_unrelated_types_is_rejected`
  in `crates/plinth-compiler/tests/lang.rs`. Removed the workaround in
  `examples/budget`: `Category` (`app/types.ts`) is a real string
  literal union again instead of a plain `string` alias, and
  `app/transactions.tsx` narrows the `Picker`'s `string` signal value
  with `draftCategory() as Category` / `editCategory() as Category`
  before calling `addTransaction`/`updateTransaction`; `app/stats.tsx`
  now takes `Category` instead of `string`. Verified with
  `npx -y -p typescript@7 tsc -p .` (clean) and
  `cargo test -p plinth-compiler --test apps`.
- #2 (fixed): `{arr.map(x => <Row .../>)}` (optionally preceded by
  `.filter(...)`) is now allowed as a JSX child. The checker
  (`check/jsx.rs::map_child`, called from `children()`'s
  `ChildKind::Nodes` case before the generic "an array of elements is
  not a child" diagnostic) recognizes this exact shape — a `.map` call
  whose single argument is an inline arrow/function — and desugars it
  to a `<List>` control built directly in TIR: `items` is the `.map`
  receiver (already a plain array; `.filter` needs no special-casing
  since it already returns one), `row` is the user's arrow
  type-checked exactly like a literal `<List>`'s `row` prop, and `key`
  is a synthetic `(item, index) => index` closure keyed by array
  position, built as a tiny AST `FuncDecl` (`index_key_fn`) so it goes
  through the normal closure-checking path and infers its parameter
  types from the offered `FuncType` like a user arrow would. Because
  the desugared control's props (`PropTarget::ListItems/ListKey/
  ListRow`) are exactly what a literal `<List>` produces, `lower.rs`
  needed no changes at all: it is the same keyed reconciler, run
  through the same runtime `list`/`run_list` path, with **no new
  runtime function**. A named function reference (`arr.map(row)`), or
  anything that is not a literal arrow/function, is unaffected and
  still rejected with `PL4004`, since the task scoped this to "an
  array expression with an arrow function returning JSX". Documented
  in `docs/language.md` ("Supported syntax"), including why a literal
  `<List>` is still the better choice when a hand-written `key` should
  track identity across a reorder rather than by position. Tests in
  `crates/plinth-compiler/tests/lang.rs`:
  `map_as_jsx_child_renders_all_items`,
  `filter_then_map_as_jsx_child_is_allowed`,
  `map_as_jsx_child_updates_on_add_remove_and_reorder` (push/reverse/
  pop via `signal.set`, checking no op errors after each), 
  `map_as_jsx_child_survives_gc_stress` (same, with
  `plinth_protocol::init_arg::GC_STRESS` on), and
  `map_with_a_named_function_reference_is_still_rejected`. Removed the
  workaround in `examples/budget/app/stats.tsx`: the `<List items={categories}
  key={(c) => c} row={...} />` wrapper around `<Progress>` rows is now
  a plain `{categories.map((c) => <Progress .../>)}`. Verified with
  `npx -y -p typescript@7 tsc -p .` (clean) and
  `cargo test -p plinth-compiler --test apps`.
- **Map/Set iteration as plain methods (gap batch 3 item 2).** `m.forEach((v,
  k) => ...)` and `s.forEach(v => ...)` now work, plus `m.keys()`/
  `m.values()`/`s.keys()`/`s.values()` as plain `K[]`/`V[]` arrays (not
  just as a bare `for…of` target, which `check/expr.rs::kv_for_of` already
  handled) — so `[...m.keys()]` and `m.values().reduce(...)` work.
  `forEach` (`map_method`/`set_method` in `check/expr.rs`) builds an index
  loop over the `keys`/`values` arrays with `CallClosure`, the same shape
  `kv_for_of` already uses for a bare `for…of`. `keys()`/`values()` return
  a copy (`arr_copy_of`, `arr_slice(arr, 0, MAX)` — the same op `.slice()`
  uses) rather than the struct's own backing array, so the caller cannot
  corrupt the `Map`/`Set` by pushing/popping the result. `entries()` as a
  plain array is not included: there is no tuple/array-of-pairs type to
  return it as (`for (const [k, v] of m)` still works). No new runtime
  function. `std/lib.d.ts`'s `Map`/`Set` declarations gained the matching
  signatures. Tests in `crates/plinth-compiler/tests/lang.rs`:
  `map_foreach_visits_every_entry_in_insertion_order`,
  `set_foreach_visits_every_value`, `map_keys_and_values_as_arrays`,
  `set_keys_and_values_as_arrays`.
- **Array `concat`/`reduce` (gap batch 3 item 3).** `a.concat(b, c)` is
  exactly `[...a, ...b, ...c]`, so it reuses the array literal's existing
  spread handling (`arr_extend`) with no new runtime function or TIR node
  — `array_method`'s `"concat"` arm just builds a `TExprKind::ArrayLit`
  with every operand marked as a spread. `reduce` only supports the
  two-argument form (callback plus an explicit initial value, matching
  the task's "reduce" audit); it is a plain index loop built from
  existing TIR (`Let`/`Loop`/`CallClosure`/`Assign`), the same approach
  `kv_for_of` and the new `Map`/`Set.forEach` use, so again no new
  runtime function. `find`, `findIndex`, `some`, `every` and `forEach`
  (also named in the task) already existed (`ArrayHof` in
  `check/expr.rs::array_method`); `sort` (with a comparator), `splice`,
  `fill` and `flat` were not implemented — see "Larger items". `std/
  lib.d.ts`'s `Array<T>` gained `concat`/`reduce`. Tests in
  `crates/plinth-compiler/tests/lang.rs`: `array_concat_joins_arrays`,
  `array_concat_on_an_empty_array`, `array_reduce_sums_with_an_initial_value`,
  `array_reduce_on_an_empty_array_returns_the_initial_value`,
  `array_reduce_with_index_builds_a_string`,
  `array_reduce_without_an_initial_value_is_rejected`.
- **Array `sort` (gap "Larger items" #1, this pass).** `arr.sort(compare?)`
  sorts in place and returns the same array (JS semantics), and is stable
  (equal keys keep their original order, required since ES2019). Lowered
  to a generated insertion sort in `check/expr.rs::array_sort` — a plain
  index loop with an inner shift loop, the same TIR shapes `reduce` and
  `kv_delete`'s shift loop already use (`Let`/`Loop`/`CallClosure`/
  `Assign(Place::Index, ..)`), so no new runtime function or codegen
  support. Insertion sort is `O(n^2)`, not `O(n log n)`; accepted because
  Plinth arrays are form-sized app state (SPEC.md §4.2), not bulk data —
  a 10,000-element sort still completes well within a test's time budget.
  Without a comparator, elements must be `string`, `number` or `boolean`
  (anything else is `PL3006: 'sort' needs a comparator for this element
  type`); `string`/`boolean` compare via the existing `TExprKind::StrCmp`
  node, and `number` also compares as a string (exact JS default-sort
  behavior, including its surprising order, e.g. `[10, 2, 1].sort()` →
  `[1, 10, 2]`) but additionally emits a new lint, `PL2025` (`sort()
  without a comparator compares numbers as strings; pass (a, b) => a -
  b`). `std/lib.d.ts`'s `Array<T>` gained `sort`. Tests in
  `crates/plinth-compiler/tests/lang.rs`: `array_sort_numbers_with_a_comparator`,
  `array_sort_returns_the_same_array_mutated_in_place`,
  `array_sort_numbers_without_a_comparator_compares_as_strings_and_warns`,
  `array_sort_strings_without_a_comparator`, `array_sort_structs_by_a_key`,
  `array_sort_is_stable_for_equal_keys`,
  `array_sort_on_empty_and_one_element_arrays`,
  `array_sort_ten_thousand_elements`, `array_sort_survives_gc_stress`,
  `array_sort_without_a_comparator_on_an_unsupported_element_errors`.
  `splice`, `fill` and `flat` are still not implemented — see "Larger
  items"; `Map.entries()` as a plain array is also still not implemented
  — see "Larger items" above.

- **Array `splice`, `fill`, `flat` (HANDOFF.md §9 item 4).** The checker
  generates each one from array operations that exist (`arr_slice`,
  `arr_extend`, push, pop and index loops) in `check/expr.rs`
  (`array_splice`, `array_fill`, `array_flat`). There is no new runtime
  function and `CORE_MINOR` does not change. `splice(start, deleteCount?,
  ...items)` follows the JS rules: a negative start counts from the end, the
  count is clamped, and the call returns the removed elements. The call
  evaluates all arguments before it changes the array. `fill(value, start?,
  end?)` changes the array in place and returns it; `start` and `end` use the
  same relative index rule as `slice`. `flat()` flattens one level: a `T[][]`
  becomes a new `T[]`; on another array it returns a copy. A depth other
  than a literal `1` was `PL2000`; see the next entry. `std/lib.d.ts` has
  the three signatures (`flat` has a `this: U[][]` overload, so `tsc`
  gives the correct type). Tests: `crates/plinth-compiler/tests/collections.rs`.

- **`flat(depth)` with a depth larger than 1, and `Infinity` (branch
  `wt-compiler2`).** The result type depends on the depth, so the depth
  must be a number literal or `Infinity`; any other expression is
  `PL2000` (golden `PL2000_flat_depth`). `array_flat` repeats the
  one-level loop (`flat_once`) once for each level, and stops when the
  elements are not arrays, so a depth larger than the nesting (or
  `Infinity`) flattens all levels, as in JS. `flat(0)` is a copy. A
  fractional depth is truncated. Each level makes one new array; there is
  no new runtime function. `std/lib.d.ts` has overloads for `tsc`: depth
  0, 1, 2 and 3 exactly, and for a larger depth or `Infinity` all levels
  of an array with up to four levels. `Infinity` and `NaN` are now global
  values (`f64` constants; `declare var` in `std/lib.d.ts`). Tests:
  `flat_with_a_larger_depth_and_infinity`,
  `flat_with_a_computed_depth_is_rejected` (`tests/collections.rs`).

- **`Map.entries()` as a value, and tuples (HANDOFF.md §9 item 4).**
  `m.entries()` returns a new `[K, V][]`. A tuple type `[A, B, …]` is a
  struct with one field for each element (`Checker::tuple_struct` in
  `check/mod.rs`; the struct name is the tuple type, for diagnostics). The
  checker reads a tuple element with a number literal index (`t[0]`, also
  as an assignment target) or an array pattern (`const [k, v] = t`, also in
  callback parameters such as `es.map(([k, v]) => ...)`). An array literal
  becomes a tuple when a tuple type is expected. `for (const e of m)` now
  binds `e` to a `[K, V]` pair (before, this was an error).
  `JSON.stringify` writes a tuple as a JSON array; `JSON.parse<T>` does not
  accept a tuple type (`PL3001`). A computed index into a tuple is
  `PL2011`. No new runtime function. `std/lib.d.ts` has `entries():
  [K, V][]`. Tests: `crates/plinth-compiler/tests/collections.rs`.

- **`Map` and `Set` iteration types for `tsc` (HANDOFF.md §9 item 4).**
  The Plinth compiler accepted `for (const [k, v] of m)` before, but `tsc`
  did not (`TS2802`), so the examples could not use it. `std/lib.d.ts` now
  declares `[Symbol.iterator]()` on `Map` (`Iterator<[K, V]>`) and `Set`
  (`Iterator<T>`), and a global `Iterable`. `tsc` finds the global
  `Iterator` and `Iterable` types only when they have three type
  parameters (the ES2015 library shape), so both have `TReturn` and
  `TNext` with defaults; Plinth uses only `T`. `examples/budget` now
  groups the monthly totals with a `Map` and reads them back with
  `for…of` (CI runs `tsc` on it). Writing this example found a bug:
  `m.set(k, (m.get(k) ?? 0) + 1)` for a new key pushed the key before it
  evaluated the value, so `get` read out of bounds. `set` now evaluates
  the value first. Test: `map_set_with_a_value_that_reads_the_same_new_key`
  and `lib_d_ts_declares_map_and_set_iterable` in
  `crates/plinth-compiler/tests/collections.rs`.

- **Dynamic-length `Chart` data (HANDOFF.md §9 item 4).** Before,
  `Chart.data` (and each series' `points`) had to be an array literal, so
  the number of points was fixed at compile time. Now any expression of
  an array of structs with a `label: string` and a `value: number` field
  is accepted: a variable, a signal or computed read, a function call, or
  a `.map()` result. `check/jsx.rs::encode_chart_points_dyn` builds the
  same wire string as the literal path, with a generated loop (`arr_len`,
  index reads, `str_concat`, `json_num_str`); no new runtime function. The
  array literal path does not change. `ChartPoint` and `ChartSeriesDef` are
  now real types when imported from `plinth:ui` (before, the import was
  `PL1004`). `series` can also be any expression now (see "`Chart`
  `series` as an expression" below). `examples/budget`'s statistics screen now
  builds both charts with `categories.map(...)`. Tests: the "Dynamic
  `Chart` data" section of `crates/plinth-compiler/tests/collections.rs`,
  and a chart check in `budget_add_filter_edit_delete_and_stats`
  (`tests/apps.rs`).

- #5 (fixed): `plinth:time` gained date/time support: `timezoneOffset`
  (a new host function, `plinth:app@1.0.0`'s `time` interface, core
  1.6), `dateParts`/`weekday` and `makeDate` (Howard Hinnant's
  `civil_from_days`/`days_from_civil`, like `plinth-ui/src/calendar.rs`,
  duplicated in a new `plinth-rt/src/datetime.rs` rather than shared, to
  keep `plinth-rt` dependency-free), `formatDate`/`toISOString` (a small
  pattern language, manual string building, no `core::fmt`), and
  `parseDate` (ISO forms, `number | null` via the existing `box_f64`
  boxing `number | null` already uses). Every date function is pure and
  takes the local offset as a parameter in `plinth-rt`, so it is fully
  unit-tested without a host (fixed epochs, leap years, month/century
  boundaries, negative pre-1970 timestamps, parse→format round trips);
  the desktop runner's `timezone-offset` is injectable
  (`Guest::set_fake_timezone_offset`) for an end-to-end compiler→wasm→
  wasmtime test (`date_time_round_trip_with_a_fake_offset` in
  `crates/plinth-compiler/tests/hostapi.rs`). Used in
  `examples/utility`'s new "Timestamps" tab and in `examples/budget`
  (`DatePicker` for transaction dates, `dateParts` for the "by month"
  breakdown in `stats.tsx`; grouping now uses a `Map` and `for…of`, see
  "Map and Set iteration types for `tsc`" below).

- **`try`/`catch`/`finally`/`throw` (HANDOFF.md §9 item 4, SPEC.md
  §5.6, core 1.10).** `throw` takes an `Error` (a built-in class with
  `name` and `message`, declared in `std/lib.d.ts` and built by the
  checker from a small prelude, `ERROR_PRELUDE` in `check/mod.rs`) or a
  subclass; `throw "text"` throws `new Error("text")`. A `catch`
  variable is an `Error` (narrow with `instanceof`); `catch (e:
  unknown)` and `catch` without a variable also work, other annotations
  and patterns are `PL2010`. The compiler generates the exception path
  instead of using the Wasm exception-handling proposal: a flag and a
  value in two app globals, a test after each call that a may-throw
  analysis (`codegen.rs::find_throwers`) marks, a branch to the
  innermost handler block, and `finally` blocks inlined on every exit
  (`return`, `break`, `continue`, the exception path). Each thunk (where
  the runtime calls app code) reports a pending exception through the
  new `error.report` WIT function (runtime function `uncaught`, core
  1.10) and clears it, so an uncaught exception in an event handler does
  not stop the app. An uncaught exception in the start-up code is
  reported and then traps. Runtime traps stay traps: the `as` cast
  checks now use a separate `TStmt::Trap`. An app with no `throw` has
  no extra code and keeps its old core version. The reasons are in
  `docs/language.md` ("Errors and exceptions"). Tests:
  `crates/plinth-compiler/tests/errors.rs`.

- **`async`/`await` (HANDOFF.md §9 item 4, SPEC.md §4.5, core 1.10).**
  `async` functions, arrows and function expressions return
  `Promise<T>` (a struct with `#state`, `#error`, `#waiters`,
  `#handled`, `#value`; app code cannot read the fields). Host calls
  without `done` return promises: `alert`/`confirm`/`prompt`, `fetch`,
  and `plinth:hub`'s `search`/`install`/`checkUpdates`/`update`
  (`std/*.d.ts` have both overloads; `std/lib.d.ts` declares `Promise`,
  `PromiseLike` and a `Promise` value so `tsc` accepts `async`). After
  checking, `check/asyncfn.rs` rewrites an `async` body into
  continuation closures: an `await` deep in an expression is moved out
  first with the evaluation order kept (`lin`), `if`/blocks/`try`/`catch`
  with an `await` get continuations for the code after them, and loops
  become loop closures that queue the next iteration as a microtask (no
  stack growth). Each continuation runs in a `try` that rejects the
  promise or calls the enclosing `catch` closure; `await` of a rejected
  promise throws. The microtask queue, the unhandled-rejection list and
  the drain are generated app code; the runtime only calls the drain
  after each event before the reactive flush (`set_drain`) and the drain
  reports unhandled rejections (`report`). Pending continuations are
  reachable from the host request that settles them (GC stress test).
  An `async` arrow where `void` is expected is detached. The items that
  were not done here are done now (see the next entries).
  `examples/dialogs` uses
  `await`; `web/test/run-net.mjs` awaits two fetches in the browser host.
  Tests: `crates/plinth-compiler/tests/async_await.rs`.

- **`await` in `switch`, `do…while` and `try`/`finally` (branch
  `wt/lang6`).** Before, these were the error `PL2009`. Now
  `check/asyncfn.rs` changes them:
  - A `switch` with an `await` becomes `if` statements
    (`async_switch`). The first `if` chain puts the index of the case
    that matches (or of `default`) in `$m`. Each case body runs when its
    index is at least `$m`, so control falls through. A `break` of the
    `switch` runs the code after it (`Ctx::sw`). At the end of a Wasm
    loop body, this `break` is a `continue`.
  - A `do…while` with an `await` becomes
    `first = true; while (true) { if (!first) { if (!c) break; } first = false; body }`.
    The condition can also have an `await`.
  - A `try`/`finally` with an `await` (`async_finally`) puts the
    `finally` body in a closure. The end of the `try` and `catch` blocks
    calls it. An exception sets `$ck = 1` and keeps the error in `$ce`.
    A `return`, `break` or `continue` that leaves the blocks sets `$ck` to
    2, 3 or 4 (`leave_via_finally`), and a `return` keeps its value in
    `$cv`. After the `finally` body, a second closure completes the
    pending action. Nested `try`/`finally` blocks chain.
  - Only `await` in a `case` value is still `PL2009` (new golden
    `PL2009_await_case`). The goldens of the removed messages are
    deleted; `PL2009_generator_method` replaces `PL2009_async_method`.
  - Limit (fixed, branch `wt-compiler2`): a `return` inside a
    synchronous `try`/`finally` that is inside an asynchronous
    `try`/`finally` ran the outer `finally` before the inner one. The
    same was true for `break` and `continue`, and an exception in the
    inner `finally` was lost. Now the statement that leaves sets a flag
    (`$leave`) and returns, so the synchronous `finally` blocks run
    first; the outermost of them calls the closure of the asynchronous
    `finally` when the flag is set (`Ctx::sync_fin` in
    `check/asyncfn.rs`). A `return` in the synchronous `finally` block
    itself replaces the pending one, as in JavaScript. Test:
    `a_sync_finally_inside_an_async_finally_runs_first`.
  - Tests: the "`await` in `switch`, `do…while` and `try`/`finally`"
    section of `tests/async_await.rs`.

- **Shared promise helpers (branch `wt/lang6`).** Before, each
  `Promise<T>` type had its own `then`, `resolve` and `reject`. Now one
  `then(p, k)`, one `reject(p, e)` and one `settle(p, state)` work on
  the common view `Promise<void>` (the fields before `#value` have the
  same layout in all promises, and the GC sees the real type). Each
  `Promise<T>` keeps only a small `resolve(p, v)`. Measured with
  `plinth build examples/dialogs` (three promise types): the app code
  went from 6075 B to 5673 B (−402 B, −6.6 %).

- **The `Promise` API (branch `wt/lang6`).** New file
  `check/promises.rs`, generated code only, no new runtime function, no
  new core version:
  - `p.then(f, g?)`, `p.catch(g)`, `p.finally(h)`. Each call makes a new
    promise and one waiter closure that the shared `then` registers. A
    callback that returns a promise is adopted. An exception in a
    callback rejects the new promise. Errors: `PL3001` when `onRejected`
    or the `catch` callback returns another type.
  - `new Promise<T>((resolve, reject) => ...)` (new AST node
    `ExprKind::NewPromise`). The type comes from `<T>` or from the
    expected type; otherwise `PL3007` (golden `PL3007_new_promise`).
  - `Promise.all`: for `Promise<T>[]`, one generated helper function per
    `T` (`make_promise_all`; it copies the array, counts the open
    promises, and resolves with the values in order). For an array
    literal of promises of different types, inline code that gives a
    tuple. `Promise<void>[]` gives `Promise<void>`; a `Promise<void>` in
    a tuple is `PL2012`.
  - `Promise.resolve(v)`, `Promise.resolve()`, `Promise.reject(e)`
    (`e` is an `Error` or a string).
  - `Promise.race`/`any`/`allSettled` were `PL3004` here; they are done
    now (see the entry "`Promise.race`, `any` and `allSettled`").
  - `std/lib.d.ts` declares all of these for `tsc`.
  - Tests: "The `Promise` API" section of `tests/async_await.rs` (also
    under GC stress); goldens `PL3001_promise_all`,
    `PL3001_promise_then`, `PL3001_promise_reject`,
    `PL3004_promise_static` (was `PL3004_promise_race`),
    `PL2012_promise_all_void`.

- **Tuples with optional and rest elements (branch `wt-compiler2`).**
  Before, `[number, string?]` and `[string, ...number[]]` were
  `PL2012`. Now:
  - The parser keeps a mark on each element (`ast::TupleMark`:
    required, optional, rest). A rest element that is not last is
    `PL2012` (golden `PL2012_tuple_rest_last`). The parser of `oxc`
    already reports a required element after an optional one
    (`PL1000`).
  - `Checker::tuple_type` (`check/mod.rs`) builds the tuple struct. An
    optional element is a field of type `T | null` with the `optional`
    mark. A rest element is a last field named `...` that holds the
    array; its type must be an array (`PL2012`, golden
    `PL2012_tuple_rest_array`). `[...T[]]` is `T[]`. `tuple_shape`
    gives the fixed element types, the number of required elements and
    the rest element type.
  - A literal where such a tuple is expected can leave out optional
    elements (they are `null`) and can have any number of elements and
    spreads after the fixed ones (they go into a new array). Wrong
    lengths and a spread in the fixed part are `PL3001` (golden
    `PL3001_tuple_optional_rest`).
  - `t[k]` with a number literal `k` past the fixed elements reads (or
    writes) element `k - n` of the rest array. A computed index is
    still `PL2011` (golden `PL2011_tuple_rest_index`). Destructuring
    reads the rest array the same way.
  - `t.length` works on all tuples: the number of required elements,
    plus the optional elements up to the last one that is not `null`,
    plus the length of the rest array.
  - `JSON.stringify` writes the rest elements into the same JSON array.
    A left-out optional element is written as `null` (JavaScript leaves
    it out); Plinth cannot tell a left-out element from `null`.
  - Not done: a rest pattern in destructuring (`const [a, ...r] = t`;
    array patterns have no rest at all), a rest element in the middle,
    a spread of a tuple into a rest element (`...[A, B]`), and
    `JSON.parse` of a tuple type.
  - Tests: `tuple_with_optional_elements`,
    `tuple_with_a_rest_element`,
    `tuple_with_optional_and_rest_elements_to_json`,
    `tuple_with_optional_and_rest_elements_diagnostics`
    (`tests/collections.rs`).

- **`Promise.race`, `any` and `allSettled` (branch `wt-compiler2`).**
  Generated code only, like `Promise.all`: one helper function per
  combinator and promise type (`make_promise_race_any`,
  `make_promise_all_settled` in `check/promises.rs`), no new runtime
  function, no new core version.
  - `race` registers a waiter on each promise that copies the result to
    the new promise. `resolve` and `reject` do nothing on a settled
    promise, so the later results are dropped. A losing rejection has a
    waiter, so it is not reported as unhandled.
  - `any` counts the rejections. When all promises reject (or the array
    is empty), it rejects with an `Error` with the name
    `AggregateError`. There is no `AggregateError` class and no `errors`
    array: TypeScript types them as `any[]`, and Plinth has only one
    error type.
  - `allSettled` gives `PromiseSettledResult<T>[]`, the TypeScript shape
    `{ status: "fulfilled"; value: T } | { status: "rejected"; reason:
    Error }`. It is an ordinary discriminated union of two object types
    (`settled_result`), so `r.status === "fulfilled"` narrows it. The
    names `PromiseSettledResult<T>`, `PromiseFulfilledResult<T>` and
    `PromiseRejectedResult` are built-in types. For `Promise<void>`, the
    fulfilled object has no `value` field (a field cannot have the type
    `void`).
  - An array literal of promises of different types is `PL2012` for
    these three (TypeScript gives a union of the value types, and a
    Plinth union cannot hold numbers or booleans). Golden
    `PL2012_promise_race_mixed`.
  - `std/lib.d.ts` declares all of them.
  - Tests: `promise_race_settles_as_the_first_promise`,
    `promise_any_takes_the_first_value_and_aggregates_rejections`,
    `promise_all_settled_gives_a_result_per_promise`,
    `promise_combinators_survive_gc_stress` (`tests/async_await.rs`).

- **`async` class methods (branch `wt/lang6`).** The parser now keeps
  `async` on a method, and the method body goes through the same
  transform as an `async` function. `this` is a parameter of the lowered
  method, so the continuations capture it like any other variable.
  Overrides and `super.m()` work. Generator methods are still `PL2009`.
  Test: `async_class_methods_capture_this_and_await_each_other`.

- **`Chart` `series` as an expression (branch `wt/lang6`).** Before,
  `series` had to be an array literal. Now any expression of an array of
  structs with a `name: string` and a `points` array of
  `{ label, value }` structs is accepted (a `ChartSeriesDef[]` variable,
  a `computed`, a `.map()` result). `check/jsx.rs::encode_chart_series_dyn`
  builds the same wire string with a generated loop; `encode_each` is the
  loop that `data` and `series` share. Tests:
  `chart_series_from_a_computed_map_updates`,
  `chart_series_from_a_variable_and_wrong_types`
  (`tests/collections.rs`), golden `PL3001_chart_series`.

- **Error banner on the desktop (branch `wt/lang6`).** Before, the
  desktop host only logged an `error.report` message. Now `GuestPort`
  has `take_errors`, `PlinthRoot` collects the errors after each commit,
  and the host shows the last one in a banner with a "Dismiss" button
  (AccessKit roles `Alert` and `Button`). The banner shows how many other
  errors there are. The app keeps running. The web host did not change
  (it calls `reportError`). Test:
  `crates/plinth-shoot/tests/error_banner.rs` (headless GPU).

## Found later

- **Narrowing on member expressions (fixed, gap batch 3 item 1).**
  `if (r.subtitle !== null) { use(r.subtitle); }`,
  `r.subtitle !== null ? r.subtitle : "x"`, and `if (r.subtitle === null)
  return;` followed by a use later in the block now narrow the member path,
  not just a plain local. `r.subtitle ?? "x"` and `r?.subtitle ?? "x"`
  needed no change: `??`/`?.` already build their own null check
  independent of the narrowing map. Implementation
  (`crates/plinth-compiler/src/check/{mod,stmt,expr}.rs`): the narrowing
  map's key widened from `VarId` to `(VarId, Vec<u32>)` (a root variable
  plus up to two struct-field indices, i.e. `a.b`/`a.b.c`); a new
  `narrow_path` builds this key by walking `Field`/`Retag` chains back to a
  `const`/parameter root, reused by `narrowable` (null narrowing),
  `instanceof` and `UnionIs` narrowing, and by `property()`'s struct-field
  read (which now applies the lookup the same way `var_read` always did).
  Dropped conservatively: `invalidate_member_narrowing` clears every
  non-empty-path entry whenever an assignment's target is not a plain
  variable, or whenever any call (`Checker::call`, wrapping the renamed
  `call_impl`) is checked — not just a call that could reach the narrowed
  path, since the checker does no alias analysis. No new runtime function;
  this is purely a type-checking change. Along the way, fixed a latent bug
  that made this (and `instanceof`/discriminant narrowing) silently not
  apply to a closure's own parameters: `Checker::closure`
  (`check/mod.rs`) only wrote the closure's `FuncDef.params` *after*
  checking its body, so any narrowing lookup during the body that needs
  `is_param` (to tell a parameter from a captured, possibly-reassigned
  `let`) saw an empty list and always said "not a parameter"; moved that
  write before the body is checked. Documented in `docs/language.md`
  ("Supported syntax"), including why the rule is deliberately more
  conservative than necessary. Tests in
  `crates/plinth-compiler/tests/lang.rs`:
  `nullish_coalesce_on_a_member_expression`,
  `nullish_coalesce_on_optional_chaining`,
  `ternary_narrows_a_member_expression`, `if_narrows_a_member_expression`,
  `if_early_return_narrows_a_member_expression_for_the_rest_of_the_block`,
  `if_early_return_narrows_a_member_expression_inside_a_closure`,
  `two_level_member_path_is_narrowed`,
  `member_narrowing_is_dropped_after_a_call`,
  `member_narrowing_is_dropped_after_an_assignment`. Removed the
  workaround in `examples/big-list/app/rows.tsx` (the `const subtitle =
  r.subtitle;` hoist before the `if (subtitle === null)` check): the row
  renderer now narrows `r.subtitle` directly. Verified with
  `npx -y -p typescript@7 tsc -p .` in `examples/big-list` (clean) and
  `plinth.exe build examples/big-list`.


## Found by 7GUIs tasks 1-5 (2026-10-06)

Found while building `examples/7guis/` (docs/VALIDATION.md V1). Tests:
`crates/plinth-compiler/tests/sevenguis.rs` (wasmtime) and
`web/test/run-7guis.mjs` (web host code in Node).

Fixed:

- **A one-way `value` came back in its own `change` event.** With
  `value={t()} onChange={(v) => t.set(v)}` (the only way to have a field
  plus a handler, because a signal `value` with `onChange` is `PL4005`),
  the prop effect sent the typed text back to the host on every key. The
  hosts dropped it (the desktop compares with `last_synced`, the web with
  the input's value), so there was no loop, but a host that handles
  events late could move the caret or lose a key. Now the runtime keeps
  the node and value of the running `change` event and `set_prop` does
  not send the same `value` back (`plinth-rt/src/ui.rs`, `host_value`).
  A node that the same event makes again with the same id (ids are used
  again, for example a list row) gets the value. Tests:
  `temperature_converts_both_ways_and_keeps_invalid_input` and
  `one_way_value_is_not_echoed_but_a_new_row_gets_it` (`tests/lang.rs`);
  `run-a11y.mjs --typing-only` (big-list toggle) found the id case.
- **`Picker options={[A, B]}` with `const` names was rejected**
  (`PL3001: each item must be a string literal`): the literal fast path
  took every array literal. Now only an array of string literals takes it;
  other arrays are joined at run time (`check/jsx.rs`). Test:
  `picker_options_from_an_array_of_const_names` in `tests/lang.rs`.
- **`TextField.error` did not show on the web**, and on the desktop an
  empty `error` gave a red border with no message. The web renderer now
  shows the message under the input, with `aria-invalid` and
  `aria-describedby`; both hosts treat `""` as no error. The desktop also
  treats an empty `Row.trailing` as none (its accessible name was
  `"title, "`).

Open (workarounds in the apps):

| # | Area | What failed | Workaround | Suggested fix |
|---|---|---|---|---|
| 7G-1 | UI API | **Fixed (UI API 1.8, 2026-10-06):** `disabled` on `TextField`, `TextArea` and `DatePicker`; the flight booker disables the return date for a one-way flight. | — | — |
| 7G-2 | UI API | **Fixed (UI API 1.8, 2026-10-06):** `Row.selected` (AccessKit `selected` on the desktop; `aria-current` and a selected background on the web, because `aria-selected` is not allowed on a list item). CRUD uses it. | — | A `List` selection prop for single and multiple selection is still open. |
| 7G-3 | Checker | **Fixed (2026-10-07):** every optional prop (string, string-literal, enum, number, size and boolean) takes `T \| null`; the core sends `Null` (a null string pointer for a string or number or bool prop, enum id -1 for an enum prop), and both host trees remove the prop, so the control shows its default. Tests: `an_optional_prop_can_be_left_out_by_a_condition`, `an_optional_number_or_bool_prop_can_be_left_out_by_a_condition`, `a_required_prop_still_rejects_null`, `a_required_prop_still_rejects_null_for_number_and_bool` (`tests/lang.rs`), `run-tree.mjs`. Before: an optional prop could not be left out by a condition: `icon={c ? "check" : undefined}` is valid for `tsc` but is `PL3001: type "check" \| null is not assignable to ...`. | Use a prop that accepts `""` (see 7G-2). | Accept `T \| null` for an optional prop, and send a "remove prop" (or the default) for `null`. Needs a host op or a default value for each prop. |
| 7G-4 | Std (`plinth:time`) | `parseDate` reads only ISO text; `makeDate(2027, 4, 31)` rolls over to 1 May, as JS does. | `examples/7guis/flight-booker/app/dates.ts` splits `DD.MM.YYYY` and checks the day with a `dateParts(makeDate(...))` round trip. | Low. A `parseDate(text, pattern)` would remove 30 lines. |
| 7G-5 | Tests | The Node tests cannot hold a mouse button or type keys; VALIDATION asks for held clicks on the timer. | Not covered for 7GUIs. `run-a11y.mjs` covers held clicks and typing on other apps. | Add the 7GUIs apps to `run-a11y.mjs`. |

## Found by 7GUIs task 6, Circle Drawer (2026-10-06)

`examples/7guis/circle-drawer`: `Canvas` with pointer events (UI API
1.12), a `Sheet` with a `Slider`, and undo and redo as snapshots of an
immutable list. Tests: `circle_drawer_draws_adjusts_and_undoes`
(`tests/sevenguis.rs`), `circleDrawer` (`run-7guis.mjs`), real clicks in
`circle_drawer_draws_where_clicked` (`plinth-shoot`, writes
`target/shots/circle-drawer.png`) and in `checkCircleDrawer`
(`run-a11y.mjs`, Edge).

| # | Area | What failed | Workaround | Suggested fix |
|---|---|---|---|---|
| 7G-6 | UI API | No secondary button: pointer events are for the main button only, and there is no context menu. The task opens the diameter dialog with a right-click on a circle. | An "Adjust diameter" button for the selected circle. | A `button` argument (or `onContextMenu`, also for a long press on touch) and an anchored `Menu` at a point. |
| 7G-7 | UI API | A `Dialog` holds only a title, a message and actions: no slider. | The slider is in a `Sheet`. | Children in `Dialog` (a small form). |
| 7G-8 | Canvas | **Fixed (UI API 1.13):** `strokeRect` and `strokeCircle` (compile-time encodings, kinds "R" and "C"); the circle drawer uses them. Before: no stroke shapes: a circle outline is two filled circles. | A border-color circle with a smaller fill circle on it. | `stroke` and `strokeWidth` on `circle` and `rect` (compile-time encoding, no core change). |

## Found by 7GUIs task 7, Cells (2026-10-06)

`examples/7guis/cells`: 26 x 100 cells as Level 2 boxes in two `Scroll`s,
one formula signal and one `computed` value for each cell, a recursive
descent parser (`app/formula.ts`) and a cycle check on the formulas. Tests:
`cells_formulas_propagate_and_find_cycles` (`tests/sevenguis.rs`), `cells`
(`run-7guis.mjs`), `cells_scroll_frame_times` (`plinth-shoot`, wheel events
and `Window::draw` times).

| # | Area | What failed | Workaround | Suggested fix |
|---|---|---|---|---|
| 7G-9 | Checker | A recursive union type alias (`type Expr = {...} \| { args: Expr[] }`) is `PL2012`. | **Help added:** each variant is an `interface`, and the alias is a union of them (an interface can name the alias). | Recursive aliases of object literal types. Low: the interface form is normal TypeScript. |
| 7G-10 | Checker | **Fixed:** a narrowed union (`Num \| Call` after `kind !== "range"`) was not assignable to the full union. A subset union is now a `Retag` (no code). Test: `a_narrowed_union_converts_back_to_the_full_union`. | — | — |
| 7G-11 | UI API | No edit in place: the task edits a cell on a double-click. | A formula field above the sheet (select a cell, edit, Enter or Set). | A small text field that the app can put in a cell (Level 2 `Input`), and `onDoublePress`. |
| 7G-12 | Desktop performance | **Fixed for scrolls (2026-10-06).** The desktop renderer built, laid out and painted every element on every frame: 2,600 cells (about 5,500 elements) took 855 ms a scroll frame in a debug build and 85.6 ms in a release build. Now a column `Scroll` with 40 or more fixed-height children builds only the rows near its viewport, and a row with 16 or more fixed-width children in a horizontal `Scroll` builds only the columns in view (spacers keep the scroll range; `culled_scroll_children`, `row_children` in `render.rs`). Cells: about 230-300 of 2,600 cells built; release build 8.6 ms a frame (AccessKit off), 12.4 ms (AccessKit on, p95 16.4 ms). | Give the rows a fixed `height` (and the cells a fixed `width`). | Still open: caching the elements of unchanged nodes for screens that do not scroll (HANDOFF §9 step 6); the accessibility tree has only the built rows, as for a virtual `List`. |

## Games (Pong, 2026-10-06)

Found while building `examples/pong` (a one-player Pong against a
computer paddle) with what exists today: the Level 2 primitives (UI API
1.6), `setInterval` from `plinth:time`, and signals. The rules are a plain
module (`app/game.ts`, 198 lines, no UI import); the court and the loop
are `app/main.tsx` (182 lines). Tests: `web/test/run-pong.mjs` (web host,
fake timers) and `pong_plays_a_point` in
`crates/plinth-compiler/tests/apps.rs` (wasmtime, fake clock).

**Measurements.**

| What | Value |
|---|---|
| `.plnt` size | 4629 B (app code 11067 B before compression) |
| Timer interval | 16 ms (a 60 Hz target) |
| One tick on the web host (Node, event + game step + commit + tree apply, no DOM) | 0.05 ms median, 0.1 ms p95 |
| One tick in wasmtime (debug build, event + commit + tree apply) | 0.08 ms median, 0.09 ms p95 |
| Desktop render of the Pong tree (`PLINTH_TRACE_RENDER=1`, debug build, element build only) | 0.35-0.6 ms per frame |
| Ticks delivered per second, desktop (`plinth run`, debug build, measured by the app with `monotonicNow`) | 45-51 in the first two seconds, then 60-62 |
| Ticks delivered per second, web host code with real timers (Node) | 61 |
| Ops per tick | 13, of which about 4 change a value (see G8) |

So the game logic and the op stream are far below the frame budget; the
frame rate is set by the timer delivery, not by Plinth's cost.

| # | Missing feature | What the game needed | Workaround used | Cost of the workaround |
|---|---|---|---|---|
| G1 | **Fixed (UI API 1.9, 2026-10-06):** `position="absolute"` with `top`/`left`/`right`/`bottom` insets in spacing units on Level 2 boxes. Pong places the ball, the paddles and the net with it (no spacers, one ball, overlap works). | — | — | — |
| G2 | **Display part done (UI API 1.10, 2026-10-06):** `Canvas` with `rect`, `circle`, `line`, `canvasText` in a view space that scales to the width. Open: pointer input (U3), so Pong still uses boxes for the court; paths and curves. | — | — | — |
| G3 | **Fixed (UI API 1.9, 2026-10-06):** `onKeyDown`/`onKeyUp` on `Box`, `Pressable`, `Scroll` (the same key names on both hosts, no repeats; the element is a tab stop). Pong: hold the arrow keys or W/S on the court, Space starts and pauses. | — | — | — |
| G4 | **Done (UI API 1.12, core 1.12):** `onPointerDown`/`onPointerMove`/`onPointerUp` with `(x, y)` on boxes and `Canvas`; a drag keeps the pointer; Pong aims the paddle with a drag. Open: a modal overlay does not block pointer events to the boxes under it on the desktop (the layer checks only its bounds), and there is no `onDrag`/drop target yet. **Pointer move and drag** (U3 `onDrag`) | Drag the paddle, or follow the pointer or a finger over the court (the natural control on a phone). | None (the buttons of G3). | On a phone the game needs three buttons below the court instead of a touch on the court. |
| G5 | **Done (core 1.12):** `onFrame((dt) => ...)` and `cancelFrame(id)` in `plinth:time`; Pong uses it. **An animation-frame timer** (a callback per display frame with the frame time, like `requestAnimationFrame`, paused when the window is hidden) | Move the game once per frame, by the real time since the last frame. | `setInterval(tick, 16)` and a fixed time step: each tick moves the game by the same amount. | The desktop host polls timers every 15 ms and gives at most one firing per poll, so ticks are not in step with the display (45-62 per second measured). The game runs slower when ticks are late (fixed step), or it must use `monotonicNow` for a delta and then becomes hard to test. On the web, background tabs throttle `setInterval` to 1/s. The app also has no way to know that its window is hidden, so the loop runs while nobody sees it (only "Pause" stops it). |
| G6 | **Done (core 1.12):** `Math.random()` (SplitMix64; the host gives the seed in the `init` record `RANDOM_SEED`, fresh entropy by default, a fixed value in tests) and `seedRandom(seed)`. Pong serves at random angles; its tests give seed 1. **Random numbers** (`Math.random` or a seeded generator in `plinth:core`) | A random serve angle. | A fixed list of 8 serve angles in `game.ts`. | Every game is the same. (The fixed list made the tests deterministic, so a seeded generator is the better API.) |
| G7 | **Sound** (a short sound from package assets) | A blip on each hit and a tone on a point. | None. | A silent game. |
| G8 | **Fixed (2026-10-06):** the runtime (`plinth-rt` `ui.rs`, `Ui::sent`) remembers the last value of each prop and text of each node and sends no op for the same value again; `value` is always sent (the host can change it). Pong: 9.0 → 1.4 ops per tick, tick 0.027 → 0.015 ms (Node). Test: `unchanged_props_and_text_are_not_sent_again` in `tests/lang.rs`. | — | — | — |
| G9 | **Scale to the window** (width-class styles, U2, or a size that fills the parent while the content keeps its aspect ratio) | A court that fills the window on a desktop and fits a phone. | A fixed court of 79 x 48 units (316 x 192 px), sized to fit the compact window. | On a wide window the court is small in a large card (see `plinth-shoot` `screen0-wide.png`). Fractions (`"1/2"`, `"full"`) exist for sizes, but the spacers need units, so a court with a fraction width cannot place the ball. |
| G10 | **Done (core 1.12):** `onCleanup(f)`, `isActive()` and `onMount(f)` in `plinth:ui`; the hosts send the `lifecycle` event (desktop: window activation; web: `visibilitychange`, focus and blur). Pong pauses in the background and stops its loop in `onCleanup`. Not tested live on the desktop (no window activation in headless gpui). `onMount(f)` (fixed 2026-10-07): runs `f` once, after the current render's elements are in the ops that `commit` sends; `plinth-rt`'s `settle` drains the queued mount callbacks after each flush, then flushes again (a mount callback can write a signal). A component inside a condition or a list row mounts again, paired with its own `onCleanup`, each time it reappears. Tests: `on_mount_runs_after_the_commit` (`tests/apps.rs`). **Lifecycle hooks** (`onMount`/`onCleanup` for a component, window focus and visibility) | Stop the loop when the screen goes away or the window loses focus; pause the game automatically. | The timer id is kept in a signal and cleared on Pause and at the end of the game. | A loop that runs until the user presses Pause, also in a hidden window. |

**Jumpy ball (owner, 2026-10-06).** At about 63 ticks per second the ball looks a little jumpy. Cause 1: positions are whole spacing units, so a ball at 0.9 units per tick moves 4 px on most ticks and 0 px on about one in ten. Cause 2: the desktop host polls timers every 15 ms and fires at most one per poll, so the gaps between frames are uneven. The plan is in `HANDOFF.md` §9 (fractional positions or Canvas, an animation-frame timer, a deadline-based timer loop, host-side motion). **Cause 2 is fixed too (core 1.12):** Pong runs on `onFrame` and steps the game by the frame time (`step(game, move, dt / TICK_MS)`), so the ball moves the same speed at any display rate and in step with the display. Measured on the owner's Windows machine (debug build, a test app that logs `dt`): about 98 frames per second, mean `dt` 10.17 ms, min 8.0 ms, max 12.1 ms after the first second (the old 15 ms timer poll gave 45-63 ticks per second). The timer loop wakes at the next deadline too (HANDOFF §9 step 3): a 16 ms `setInterval` now has gaps of 14-18 ms (before: 15-26 ms), and a late interval skips missed ticks instead of a burst of firings 1 ms apart. **Cause 1 is fixed (UI API 1.11):** sizes and insets take fractional spacing units, Pong has no `cell()` any more, and the tests on both hosts check even steps of 0.9 units per tick. The cost: the ball's position changes on each tick, so the web test counts 2.4 ops per tick (1.4 before).

No compiler, runtime or renderer bug was found: the app compiled on the
first try, and both hosts gave the same layout facts and the same game.

## Found later (2026-10-06, coordinator)

- **Rest patterns in array destructuring: done.** `const [a, , ...r] = xs` on arrays and on tuples with a rest element (`tests/lang.rs` `array_rest_patterns_bind_the_remaining_elements`, golden `PL2000_tuple_rest_pattern`). **Rest in object patterns and default values in patterns: done (2026-10-07).** `const { a, ...o } = x` (a new object of an anonymous object type with the other fields) and `{ a = 1 }`, `[a = 1]`, parameter patterns, nested patterns and `for…of` patterns. A default applies to `null` and, in an array pattern, past the end (a length check, no trap). Tests: `object_pattern_defaults_*`, `array_pattern_defaults_apply_past_the_end`, `object_rest_*` (`tests/lang.rs`).
- **`join` on number and boolean arrays: done** for a literal separator (the default `","` too); a computed separator on a number array is still `PL3001`. Test `join_converts_numbers_and_booleans`.

## Async code size (2026-10-07)

`async_code_size` (`crates/plinth-compiler/tests/async_size.rs`, `--ignored`) compiles the same logic with `await confirm(...)` and with `confirm(..., done)`. App module bytes above an app without dialogs (1,103 B):

| Awaits | `async` | Callbacks |
|---|---|---|
| 1 | +1,653 B | +235 B |
| 2 | +1,965 B | +314 B |
| 4 | +2,600 B | +472 B |
| 8 | +3,902 B | +788 B |

So `async` costs about 1.35 KB one time (the promise machinery, the microtask queue and the drain are generated app code) and about 320 B for each `await` (a continuation closure and its `try` wrapper), against about 80 B for each callback. Proposal (low priority; app size is a goal, not a cap): move the promise machinery into the core (no size limit, HANDOFF decision 6) to save the fixed part in every async app, then share one wrapper for the continuations.

## Desktop frame cost (2026-10-07)

`crates/plinth-shoot/tests/frame_cost.rs` draws 20 frames of every example in a headless window (1000 x 800) and prints the median of `Window::draw`; a second test measures five of one control. Release build, the slowest first: cells 8.3 ms (5,666 nodes, culled scroll), **settings-gallery 7.1 ms with only 26 nodes**, utility 3.6 ms, budget 3.4 ms; big-list 1.6 ms with 21,008 nodes (a virtual `List`). So the cost does not follow the node count.

| # | Finding | Next step |
|---|---|---|
| F1 | **Partly fixed (2026-10-07).** The labelled form controls were expensive: five `Picker`s 2.8 ms, `DatePicker`s 3.6 ms, against 0.36 ms for five `Text` (release). An interleaved probe (variants switched draw by draw, so CPU load from other builds cancels out; five Pickers, debug) found the cause: each nested `flex`/`flex_col` box around the input about doubles the layout work of the input's subtree (input alone 6.1 ms; row plus a block input column 15.8 ms; row plus a `flex_col` input column, the old wide layout, 37.8 ms; the old compact column of columns 57.4 ms). `labelled` (`render.rs`) now uses block layout for the input column and for the compact stack; the screenshots of settings-gallery and budget are byte-identical before and after. Five of each, debug: Picker 38 → 16 ms, Slider 26 → 15 ms, DatePicker 30 → 16.5 ms, NumberField 18-27 → 12 ms. | The cause is in gpui-ce: `taffy.rs` `compute_layout` runs for each element that asks for layout as a root, and when the node had a layout already it clears the cached bounds of its subtree and computes again (`previous_space.is_some()`); the Taffy tree is also built again for each frame, so Taffy's cache lives one frame only. Fix it in the gpui-ce fork (keep nested flex linear), then remove other needless flex boxes in `render.rs`. |

## Robustness (2026-10-07)

A pass on stability: the compiler must never crash on any source text, and a bad app must stop only itself, with a plain reason, on every host. Tests: `crates/plinth-compiler/tests/robustness.rs` (a mutation test over the example sources: lines deleted, duplicated and swapped, spans removed, tokens inserted, names swapped; 300 cases in the check script, `PLINTH_FUZZ_ITERS`/`PLINTH_FUZZ_SEED` for more; a stack overflow writes the case to `target/fuzz-failures/current.txt`), the "Limits" tests in `tests/errors.rs`, `web/test/run-stop.mjs` and `checkStopBanner` in `run-a11y.mjs`.

Fixed:

- **`break` outside a loop or a `switch` was accepted**, and codegen then panicked (a subtract with overflow). The checker counts the `switch` statements around a statement (`FnCx::switches`). Found by the mutation test.
- **`JSON.stringify` and `JSON.parse<T>` of a recursive type overflowed the compiler's stack** (`interface Node { kids: Node[] }`): the JSON code was inline for each type. Now each struct type has one generated function for each direction (`json_func`), which can call itself; a required field of the type itself (`{ again: Loop }`) parses to `null`. Tests: `json_round_trips_a_recursive_type`, `json_parse_of_a_type_that_holds_itself_is_null`. Found by the mutation test.
- **Deep nesting overflowed the compiler's stack.** A debug build stopped at a sum of 60 terms (`1+1+...`), 100 parentheses or 50 nested array literals, on the 1 MiB main thread of Windows. Now every compile runs on a 256 MiB stack on the same thread (`stacker`, `plinth_compiler::with_stack`), and the parser stops at 1000 levels with `PL1000: this code nests more than 1000 levels deep` (`parse::MAX_DEPTH`). Test: `deep_nesting_builds_up_to_the_limit_and_is_a_diagnostic_past_it`.
- **"This app stopped" showed a long wasm backtrace** (20 lines of `<unknown>!<wasm function N>` for a stack overflow). The runner now puts a plain reason at the top of the error (`stop_reason`): the core's `trap: <reason>` log, else the trap kind. The desktop banner shows only that line; the log keeps the detail.
- **Out of memory had no reason**: a failed `memory.grow` gave a null pointer to Rust's allocation error path, which reached the panic handler (and the over-aligned path used the null as an address). The allocator now stops with `trap: out of memory` (`trap_oom`, no allocation).
- **The web host had no stopped state and no banners.** A trap left a broken instance that timers and events still called, and an uncaught error went only to `console.error`. Now `PlinthApp` stops after a trap (`stopped`, `onStop`, the same reasons as the desktop) and never calls the app again, and `DomRenderer` shows "This app stopped" (the screen becomes `inert`) and "An error occurred" with Dismiss, as the desktop does.

Long use (`crates/plinth-compiler/tests/monkey.rs`, 2026-10-07): every example gets 400 random UI events in the desktop runner (presses, typed text, toggles, slider values, keys, pointer moves), with timers, frames and dialog answers between them, then 100 more in GC stress mode. It checks that the app never traps, that every commit applies with no op error, and that the memory stops growing (`Guest::memory_bytes`, new). It found three memory bugs in the core, all fixed:

- **A large block lost its tail at each reuse.** The allocator took a free block up to two times the requested size, but `dealloc` knows only the requested size, so the rest was lost. Now the rest goes back on the free list as its own block.
- **First fit grew the memory at each collection.** The newest free block (at the head of the list) was often up to two times too large for a request, so the next request of that size found nothing and grew the memory. The GC's own mark stack (doubled up to 128 KiB for 26,500 objects) grew the 7GUIs cells by 128 KiB at each collection: in GC stress mode it ran out of memory after about 450 events. Now the allocator uses best fit.
- **Array data did not count toward the next collection.** Only GC objects counted toward the 1 MiB threshold, so `big-list` (filters of 10,000 rows) used memory with no collection and ran out after about 780 events. Now array buffers count too (`gc::buffer_alloc`).

After the fixes, all 27 examples keep a level memory in both modes (cells 7.4 MiB, big-list 22.2 MiB).

Hostile input to the host (no bug found; the tests stay as guards):

- **The op stream** (`crates/plinth-ui/tests/hostile_ops.rs`): random well-formed ops over a small id range (bad parents, cycles, removed nodes, wrong value types) and their bytes with random changes. After each commit the tree keeps its invariants (each child names its parent, no node in two places, no cycle). 20,000 commits passed.
- **The renderer** (`crates/plinth-shoot/tests/hostile_render.rs`): random trees of real controls with wrong prop types, `NaN`, huge numbers, unknown enums, bad canvas shapes and long text, at three widths, with later moves, removes and navigation. 150 windows drew with no panic (20 in the check script, about 45 s).
- **The linker** (`mutated_app_modules_never_panic_the_linker` in `robustness.rs`): a valid package can hold any `app.wasm`, so changed app modules go through `split::link_app` and wasmtime. 3,000 cases: 97 linked, 80 started, no panic.

Open:

| # | Area | What | Suggested fix |
|---|---|---|---|
| R1 | Web host | **Fixed (2026-10-07).** Each app runs in its own dedicated Worker (`web/app-worker.js`, step 1), and the proxy in the document (`WorkerApp`) has a watchdog (step 2): a call that does not end in 2 s (`CALL_TIME_LIMIT_MS`) ends the Worker, and the app stops with "the app is not responding (time limit reached)", as on the desktop. The page stays responsive during the loop (`run-worker.mjs`, `checkStopBanner` in `run-a11y.mjs`). | — |
| R2 | Web host | **Fixed (2026-10-07).** `loadCore` caps the core's memory at 64 MiB (`capCoreMemory`, `MEMORY_LIMIT_PAGES`); a runaway allocation stops the app with "out of memory" (`web/test/run-stop.mjs`). | — |
| R3 | Compiler | A sum of more than 1000 terms is `PL1000`; `tsc` accepts it. | Convert left-deep binary chains without recursion (an explicit stack in the parser, checker and codegen). Low: hand-written code does not get near the limit. |
| R4 | Tests | The mutation test only changes valid programs, at the text level. | Add structure-aware changes (swap two expressions of the same kind, change a type annotation) and a nightly run with a large `PLINTH_FUZZ_ITERS`. |

### R1/R2 design (2026-10-07; R1 and R2 implemented)

Looked at giving the web host the desktop's two limits (2 s per `init`/`on-event` call,
64 MiB memory, the same plain stop reasons). R2 is done. R1 step 1 (the Worker split)
is done; see "R1 step 1, as built" below the table. R1 step 2 (the watchdog) is done;
see "R1 step 2, as built".

**R1, the time limit.** The core must run off the document's main thread, because the
main thread is the only place that can forcibly stop a stuck computation (`Worker.
terminate()`; there is no such thing for a function call on the same thread as the DOM).
So: each app's `PlinthApp` (today `plinth-web.js`, used by `app-frame.js` and the Node
tests) moves into a dedicated `Worker`, one per app. The document that owns the DOM
(`app-frame.js`'s `container`, `dom-renderer.js`'s `DomRenderer`) keeps a small proxy
object with the same shape `PlinthApp` has today (`init()`, `onEvent()`, `onCommit`,
`onStop`, `onError`, `stopped`), so `DomRenderer` and `app-frame.js` barely change: the
proxy posts a message to the Worker instead of calling the core directly, and relays the
Worker's messages back as `onCommit`/`onStop`/`onError` calls.

The watchdog lives in the proxy, on the main thread: before posting an `init` or an event
to the Worker, it starts a 2 s timer; the Worker answers with a `"call-done"` message
after it returns from that call (trap or not), which clears the timer. If the timer
fires first, the proxy calls `worker.terminate()` and reports the stop itself, with the
same reason the desktop gives for its epoch interrupt: "the app is not responding (time
limit reached)". This matches the desktop's rule: the limit is per call, not per session
(SPEC §10.5), and a slow `init` is covered the same as a slow `on-event`.

Each host import in `plinth-web.js`'s `hostImports`, checked for whether it still works
from inside a Worker:

| Import | Works from a Worker as is | Notes |
|---|---|---|
| `ui.commit` | Yes, over `postMessage` | The op buffer already crosses a boundary for the sandboxed frame today (frame → parent); it crosses one more (Worker → document). Transfer the `Uint8Array`'s buffer, do not clone it. |
| `dev.log`, `error.report` | Yes, over `postMessage` | Fire-and-forget, as today. |
| `time.now`, `time.monotonic-now`, `time.timezone-offset` | Yes, unchanged | `Date` and `performance.now()` exist in a Worker. |
| `time.set-timer`, `time.cancel-timer` | Yes, unchanged | `setTimeout`/`setInterval` exist in a Worker. |
| `time.set-frame-timer` | No, needs a proxy | `requestAnimationFrame` does not exist in a plain dedicated Worker. The Worker asks the document for frame ticks (one `postMessage` request, one `dt` message back per frame) instead of calling `requestAnimationFrame` itself. |
| `store.kv-*` | No, needs a cache | `localStorage` does not exist in a Worker. Reuse the pattern the sandboxed Hub frame already uses for its own kv (`hub-storage.js`'s `frameKvStore`): the document sends a full kv snapshot when the Worker starts; the Worker answers reads from that cache (so `kv-get`/`kv-keys` stay synchronous) and posts writes to the document to persist, fire-and-forget. |
| `dialog.alert/confirm/prompt` | Yes, unchanged shape | Already a request-id-now, completion-later call (`askDialog`); the completion now arrives from the document over `postMessage` instead of a resolved Promise. |
| `net.fetch` | Yes, unchanged shape | Same request/completion shape; `fetch` would even work inside the Worker directly, but routing it through the document keeps the one already-checked `net:` capability path and the sandboxed frame's "the parent makes the request" rule (docs/web-hub.md §4) in one place. |
| `files.*` | Yes, unchanged shape | Same request/completion shape, relayed to the document (which owns the `FileSpace`/IndexedDB). |
| `clipboard.write-text` | Yes, unchanged | Already fire-and-forget. |
| `clipboard.read-text` | No, needs a cache | `navigator.clipboard` does not exist in a Worker. Same fix as today's synchronous-cache trick, one level further out: the Worker keeps `clipboardCache`, returns it synchronously, and the document posts a fresh value back after its own async `navigator.clipboard.readText()` resolves. |
| `hub.*` | Yes, unchanged | `FrameHub` (`hub-host.js`) is a plain JS object over a JSON snapshot, with no DOM access; it can run inside the Worker exactly as it runs inside the frame document today, using the same `send`/`ask` shape, now posting to the document instead of to the parent frame. |

So two imports (`store.kv`, `clipboard.read-text`) and one (`time.set-frame-timer`) need a
cache or a proxy; the rest cross the new boundary with no change in shape, because the
existing frame ↔ parent protocol (`app-frame.js` ↔ `frame-host.js`) already solved the
same problem one layer out (an opaque-origin frame that also cannot touch `localStorage`
or the clipboard directly). The Worker split adds a second, inner hop of the same kind.

Fallback for the single-file export (`plinth build --target web --single-file`,
SPEC §10.3): a `Worker` can start from a `Blob` URL built from the bundled script text
(`new Worker(URL.createObjectURL(new Blob([source], { type: "text/javascript" })))`), so
the export does not need a second file on disk. If that turns out not to work in some
target (for example a very old browser, or a page served with a strict Content Security
Policy that blocks blob: workers), the fallback is the current no-Worker code path,
kept behind a feature check (`typeof Worker === "function"`), with no time limit and no
memory cap — the same gap this item exists to close, so the fallback must stay rare.

**R1 step 1, as built (2026-10-07).** `web/app-worker.js` holds both sides:
`runWorker(scope)` (the Worker: it runs `PlinthApp`) and `WorkerApp` (the proxy in the
document, with the surface of `PlinthApp`: `load`, `init`, `onEvent`, `onCommit`,
`onStop`, `onError`, `stopped`). `startAppWorker` starts the Worker from a `blob:` URL:
in the bundle (`plinth.js`, also the single-file export) the source is the bundle itself
(`plinthBundle.toString()`, called with the mode `"worker"`), so no second file is
necessary; elsewhere it is a small classic script that loads `app-worker.js` with
`import()` and keeps the messages that come before the import ends (the web Hub serves
the web files with CORS, so the sandboxed frame can import them). The Worker is always
a classic script: headless Edge refuses a module Worker from a `blob:` URL in an opaque
origin, but `import()` in a classic Worker works there. The browser
paths (`app-frame.js`, so `<plinth-app>` and the web Hub; `index.html`;
`test/a11y.html`) use the Worker. The Node tests keep `PlinthApp` in the same thread;
`test/run-worker.mjs` runs both sides over a `MessageChannel`. There is no fallback with
no Worker. How each import works now:

- `ui.commit`: the op bytes go to the document as a transferred buffer
  (`onCommitBytes`); the proxy decodes them.
- `dev.log`, `error.report`: messages; the proxy calls `log`/`reportError` and `onError`.
- `time.*` timers: in the Worker. `time.set-frame-timer`: the Worker sends
  `frame-request`; the document calls `requestAnimationFrame` and sends the time back
  (`frame`); `frame-cancel` stops it.
- `store.kv`: a snapshot of the document's store goes to the Worker in `load`; the
  Worker answers from it, applies the quota (`frameKvStore`, when the caller gives
  `kvQuota`), and sends each write to the document's store.
- `clipboard`: `write-text` is a message; `read-text` answers from the cache of the
  Worker and asks the document for a new value.
- `dialog`, `net.fetch`, `files.*`: `ask` messages; the proxy answers with the result
  of the document's implementation (the same defaults as `PlinthApp`).
- `hub.*`: `FrameHub` runs in the Worker on the snapshot; `hub-save`, `hub-launch` and
  `hub-call` go out through the frame to the page; the page's `hub-state` comes in
  through `WorkerApp.applyHubState`.
- `init` returns a promise that resolves after the call (the Worker sends `done`).
  Events and commits now cross a thread, so a commit comes after the event handler in
  the document returns. Two checks in `run-a11y.mjs` clicked again before the commit
  of the last input arrived; they now wait for it (a person cannot click that fast).
  The big-list check now times the whole round trip until the render. The renderer
  keeps the value of a two-way bound field in its
  tree, so typing does not lose characters. One known edge case: an app that changes
  the `value` of a field in its `change` handler (for example to upper case) can get a
  late commit while the user types the next character.

**R1 step 2, as built (2026-10-07).**

- `PlinthApp` calls `onCallStart`/`onCallEnd` around each call into the app (`init`,
  and each event: UI, timer, frame, completion, lifecycle). In the Worker they post
  `call` and `done`. So the Worker also tells the document about calls that start in
  the Worker (timers, answers). A message that the Worker posts before an endless loop
  still reaches the document. The calls do not overlap, so one timer is enough.
- `WorkerApp` starts a timer of `CALL_TIME_LIMIT_MS` (2000, in `plinth-web.js` next to
  `MEMORY_LIMIT_PAGES`; the option `timeLimitMs` changes it for tests) when it reads
  `call`, and clears it at `done`. The time starts when the document reads `call`, so
  a busy document makes the limit longer, never shorter. After the limit, a grace of
  100 ms lets a `done` that already waits in the queue arrive first (the document can
  be busy with a large commit of the same call).
- At the limit: `worker.terminate()`, `stopped` = `TIME_LIMIT_REASON` ("the app is not
  responding (time limit reached)", the desktop's words), the frame loop stops, the
  answers to open host requests are dropped, an `init` that did not end resolves (so
  the frame reports "started" and shows the banner), and `onStop`.
- Tests: the stop fixture has a `loop` button. `run-worker.mjs` runs the Worker side in
  a `worker_threads` thread with a 300 ms limit (the app stops after about 400 ms) and
  checks that big-list's init (10,000 rows) is not stopped with the default limit.
  `checkStopBanner` clicks `loop` in Edge: the page answers in about 1 ms during the
  loop, and the banner shows the reason after about 2.1 s. The big-list load in Edge
  (about 850 ms) is not stopped.
- A dialog that the page shows for the app stays open after the stop; its answer goes
  nowhere.

**R2, the memory cap.** The core's wasm memory is defined inside the core module itself
(`(memory (export "memory") ...)`, no `maximum`); `plinth-web.js`'s `loadCore` just
instantiates it as is. Capping it from the web host, without changing the core build
(which also serves the desktop, where the cap is `StoreLimits`, not a wasm-declared
max), means patching the already-encoded max into the module's memory section before
`WebAssembly.instantiate`, the same kind of raw byte surgery `plinth-web.js` already
does to read the app module's table-import minimum (`readTableImportMin`): find the
memory section (id 5), and if its one memory has no max (the size flag's bit 0 is 0),
re-encode that one section with the flag set and a max of 1024 pages (64 MiB at 64 KiB
per page), splicing the new section bytes in place of the old ones (its length prefix
changes, so the bytes after it shift; sections before it do not move). A grow past the
cap then fails the way an unbounded grow past the engine's own limit already fails today
(`memory.grow` returns -1 inside the module), which is exactly the path the desktop's
`trap_oom` (`crates/plinth-rt`'s allocator) already turns into "trap: out of memory" — so
once the memory section is patched, the existing `stop_reason` plumbing in `plinth-web.js`
needs no change; the cap only needs to exist before instantiation.

## wasmi engine (2026-10-07)

Step 2 of the wasmi interpreter engine for iOS (SPEC.md §9.2; step 1 added
the engine selection, the core-module extraction, and six host interfaces
behind `PLINTH_ENGINE=wasmi` and the `wasmi` cargo feature on
`plinth-runner-wasmtime`). This pass added the remaining host interfaces,
the call time limit (fuel), and the memory cap, and ran the bulk of the
compiler's test suites under both engines.

wasmi 2 is a register-machine interpreter: ahead of a call it compiles
Wasm to its own register-based bytecode, closer to a simple bytecode VM
than a tree-walking interpreter, but still well behind wasmtime's
compiled code. Running `plinth-rt`'s own bytecode interpreter *inside*
wasmi is two levels of interpretation, which is why the gap below is
large on CPU-heavy work and small on ordinary UI event handling.

**What works:**

- Every `plinth:app` interface is wired: `ui`, `dev`, `error`, `time`,
  `store`, `clipboard` (step 1), and now `dialog`, `net`, `files`, `hub`
  (`crates/plinth-runner-wasmtime/src/wasmi_engine.rs`), each calling the
  same `HostState` method the wasmtime `bindings::...::Host` trait impls
  call. The core ABI layouts (ptr/len pairs, `retptr`-based results) match
  `web/plinth-web.js`'s `hostImports` byte for byte.
- The call time limit uses fuel (wasmi has no epoch-interruption
  primitive): `Config::consume_fuel(true)`, and `Store::set_fuel` before
  every `init`/`on-event` call, from a budget `FUEL_PER_MS *
  Limits::call_timeout.as_millis()`. Running out of fuel maps to the
  same plain reason as wasmtime's epoch interrupt: "the app is not
  responding (time limit reached)".
- The memory cap reuses `crate::Limiter` (plain `memory_cap`/`memory`
  fields, no longer wrapped around wasmtime's own `StoreLimits` type) through
  `wasmi_core::ResourceLimiter`, the same trait `wasmi::ResourceLimiter`
  re-exports; `Guest::memory_bytes` now works on both engines.
- `FUEL_PER_MS` is **not** a wall-clock-accurate conversion: real fuel
  consumption per millisecond varied about 15x between workloads measured
  here (roughly 11,000 fuel/ms for a tight add-and-branch TS `while` loop
  running through `plinth-rt`'s own interpreter, versus roughly 172,000
  fuel/ms during a 10,000-element `Array.sort`). The constant
  (`150_000`) is picked low enough that the slowest case still trips
  within a few seconds for `Limits::default()`-sized timeouts, which
  means a legitimately heavy computation (`array_sort_ten_thousand_elements`
  in `tests/lang.rs`) needs its own longer `call_timeout` under wasmi,
  added via a new `run_with_limits` test helper; it still passes well
  inside `Limits::default()` on wasmtime.

**Build note:** optimizing the `wasmi` crate itself at `opt-level = 3` in
a `[profile.dev]` package override **crashes** (`STATUS_ACCESS_VIOLATION`)
on Windows/MSVC debug builds, reproduced with `debug-assertions = false`
and `overflow-checks = false` too (so it is not those two interacting
with `-O3`; it is specific to the `wasmi` crate's own code at `-O3`
outside a full `release` profile). The workaround in the root `Cargo.toml`
optimizes `wasmi_core`, `wasmi_ir` and `wasmi_collections` (the bytecode
and numeric-op crates, where the hot loop lives) and leaves `wasmi` itself
at `opt-level = 0`; `wasmi` is mostly glue (`Store`, `Linker`, `Caller`)
so this still captures most of the speedup without the crash.

**Test results**, `cargo test -p plinth-compiler`:

| Suite | wasmtime | `PLINTH_ENGINE=wasmi` |
|---|---|---|
| `--test errors` (incl. all four "Limits" tests: endless loop, loop at start, recursion, memory) | 28/28, 6 s | 28/28, 69 s |
| `--test lang` | 171/171, 29 s | 171/171, 696 s (696 s is almost all one test, the 10k-element sort, at its own 120 s `call_timeout`) |

Also passing under `PLINTH_ENGINE=wasmi` in this pass (not re-confirmed in
the final wrap-up run, see "Left for the next agent" below): `--test
sevenguis` (7/7, 336 s, `cells_formulas_propagate_and_find_cycles` is most
of it), `--test apps` (7/7, 1.6 s, includes `pong_plays_a_point`).
`cargo test -p plinth-runner-wasmtime --features wasmi` (29/29, incl. the
two `wasmi_core::extract_core_module` unit tests) and the default-engine
`cargo test -p plinth-runner-wasmtime` (29/29) both pass.

**Left for the next agent:**

- `--test monkey`, `--test async_await`, `--test files`, `--test hub_api`,
  and `--test net` have not been run yet under `PLINTH_ENGINE=wasmi`
  (`monkey` was still running, over 10 minutes in, when this pass was cut
  short to hand off; it is long even on wasmtime). `files` and `hub_api`
  and `net` exercise the interfaces this pass just wired for the first
  time, so they are the most likely to need a fix.
- No speed comparison numbers (wasmtime vs wasmi, release build: one Pong
  tick, one Cells formula change, `init` of the 10,000-row List example)
  were produced. `crates/plinth-compiler/tests/apps.rs`'s
  `pong_plays_a_point` and `tests/sevenguis.rs`'s
  `cells_formulas_propagate_and_find_cycles` already print a timing line
  each (`eprintln!`); `tests/perf.rs`'s `#[ignore]`d `list_10k_perf` prints
  one for `init`. Running each once per engine in `--release` (`Runner::new`
  already reads `PLINTH_ENGINE`, so no new code is needed) would answer
  this without a new bench file.
- `FUEL_PER_MS` is calibrated from exactly two data points (see above); a
  third workload (e.g. one of the still-unrun suites) could easily fall
  outside the 15x spread already seen and need either a higher constant
  or another test-specific `call_timeout` override.
- `scripts/ci-local.sh full` was run once at the end of this pass with the
  default engine (wasmtime); see the commit log for the result.

## Core policy `patch` (found 2026-10-07, fixed 2026-10-07)

**Fixed:** the manifest records `core-built` (the build core version) for `core = "patch"`; `cores::patch_minor` selects on its minor version and falls back to the need for an old manifest. Checked with `plinth registry build` on an app that needs 1.0 and was built with 1.12.0 (the old code refuses it; the new code links it).


`core = "patch"` must mean "only patches of the minor version that the app was built with" (SPEC §10.5, `docs/getting-started.md` §7.1). `cores::select_patch` uses `need.1`, the lowest minor that the app *needs* (from its imports), not the minor that built it, and the manifest does not record the build core for this policy. So a small app that needs core 1.0 but was built with core 1.12 does not link on a host that has only 1.12: `plinth registry build` fails with "the app needs core 1.0 (policy `patch`)". Fix: record the build core version in the manifest for `patch` (as `core-digest` does for `pinned`; an optional field, `docs/COMPATIBILITY.md` §4), select on it, and fall back to `need` for a manifest without it. Add a test with an app that needs 1.0 and was built with 1.12. `docs/guides/updates.md` has a caution until the fix.
