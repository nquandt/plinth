// The sheet: one formula signal and one computed value for each cell. A
// value reads only the cells that its formula names, so an edit updates
// only the cells that depend on it. A cycle gives an error in every cell
// on it; it is found from the formulas, before any value is read.
import { signal, computed, Signal, Computed } from "plinth:ui";
import { Math } from "plinth:core";
import { COLS, ROWS, Expr, CellValue, parse, refs, evaluate, error } from "./formula";

export const CELLS = COLS * ROWS;

export const formulas: Signal<string>[] = [];
const parsed: Computed<Expr>[] = [];
export const values: Computed<CellValue>[] = [];

/** True if the formula of `from` reads `target`, directly or through other cells. */
function reaches(from: number, target: number, seen: Set<number>): boolean {
  const out: number[] = [];
  refs(parsed[from](), out);
  for (const c of out) {
    if (c === target) {
      return true;
    }
    if (!seen.has(c)) {
      seen.add(c);
      if (reaches(c, target, seen)) {
        return true;
      }
    }
  }
  return false;
}

for (let i = 0; i < CELLS; i++) {
  formulas.push(signal(""));
}
for (let i = 0; i < CELLS; i++) {
  const cell = i;
  parsed.push(computed(() => parse(formulas[cell]())));
  values.push(
    computed(() => {
      if (reaches(cell, cell, new Set<number>())) {
        return error("a cycle");
      }
      return evaluate(parsed[cell](), (c) => values[c]());
    }),
  );
}

/** The text that a cell shows. */
export function display(v: CellValue): string {
  if (v.kind === "num") {
    return `${Math.round(v.num * 1000000) / 1000000}`;
  }
  if (v.kind === "error") {
    return "#ERR";
  }
  return v.text;
}

/** Some cells to start with. */
export function sample(): void {
  formulas[1 * COLS + 0].set("Item");
  formulas[1 * COLS + 1].set("Price");
  formulas[2 * COLS + 0].set("Apples");
  formulas[2 * COLS + 1].set("1.5");
  formulas[3 * COLS + 0].set("Bread");
  formulas[3 * COLS + 1].set("2.25");
  formulas[4 * COLS + 0].set("Total");
  formulas[4 * COLS + 1].set("=sum(B2:B3)");
}
