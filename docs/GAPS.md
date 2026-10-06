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

- `async`/`await` and `try`/`catch`: neither app needed them (no host calls
  that return a promise in the current API surface), but any future host
  API that is naturally asynchronous (beyond the existing `plinth:dialog`
  callback style) will want them.
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
  than a literal `1` is `PL2000`. `std/lib.d.ts` has the three signatures
  (`flat` has a `this: U[][]` overload, so `tsc` gives the correct type).
  Tests: `crates/plinth-compiler/tests/collections.rs`.

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
  breakdown in `stats.tsx`; grouping uses a linear-scan array rather
  than `Map` for…of, since `std/lib.d.ts`'s `Map` is not declared
  `Iterable` for `tsc`, see "Larger items" above).

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

