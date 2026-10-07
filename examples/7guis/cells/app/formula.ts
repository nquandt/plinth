// The formula language of the cells: a parser and an evaluator, with no UI.
//
//   cell     := formula | number | text
//   formula  := "=" expr
//   expr     := term (("+" | "-") term)*
//   term     := factor (("*" | "/") factor)*
//   factor   := number | ref | call | "(" expr ")" | "-" factor
//   call     := name "(" arg ("," arg)* ")"      sum, prod, avg, min, max, add, sub, mul, div
//   arg      := ref ":" ref | expr               a range (A1:B3) only in a call
//   ref      := letter digits                    A0 ... Z99
import { Math, parseNumber } from "plinth:core";

export const COLS = 26;
export const ROWS = 100;
const LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS = "0123456789";

// The parsed formula. The recursive variants are interfaces: a type alias
// cannot name itself (PL2012), an interface can.
interface Num {
  kind: "num";
  value: number;
}
interface Ref {
  kind: "ref";
  cell: number;
}
interface Range {
  kind: "range";
  cells: number[];
}
interface Call {
  kind: "call";
  name: string;
  args: Expr[];
}
interface Bin {
  kind: "bin";
  op: string;
  left: Expr;
  right: Expr;
}
interface Neg {
  kind: "neg";
  arg: Expr;
}
interface Text {
  kind: "text";
  text: string;
}
interface Empty {
  kind: "empty";
}
interface Failed {
  kind: "error";
  message: string;
}
export type Expr = Num | Ref | Range | Call | Bin | Neg | Text | Empty | Failed;

/** The value of a cell. */
export interface CellValue {
  kind: "num" | "text" | "empty" | "error";
  num: number;
  text: string;
}

export function cellName(cell: number): string {
  return `${LETTERS.charAt(cell % COLS)}${Math.floor(cell / COLS)}`;
}

function isDigit(c: string): boolean {
  return c !== "" && DIGITS.indexOf(c) >= 0;
}

function isLetter(c: string): boolean {
  return c !== "" && LETTERS.indexOf(c.toUpperCase()) >= 0;
}

class Parser {
  text: string;
  pos: number;
  error: string;

  constructor(text: string) {
    this.text = text;
    this.pos = 0;
    this.error = "";
  }

  peek(): string {
    while (this.text.charAt(this.pos) === " ") {
      this.pos = this.pos + 1;
    }
    return this.text.charAt(this.pos);
  }

  fail(message: string): Expr {
    if (this.error === "") {
      this.error = message;
    }
    return { kind: "error", message: message };
  }

  expect(c: string): void {
    if (this.peek() !== c) {
      this.fail(`expected "${c}"`);
      return;
    }
    this.pos = this.pos + 1;
  }

  expr(): Expr {
    let left = this.term();
    let c = this.peek();
    while (c === "+" || c === "-") {
      this.pos = this.pos + 1;
      const right = this.term();
      left = { kind: "bin", op: c, left: left, right: right };
      c = this.peek();
    }
    return left;
  }

  term(): Expr {
    let left = this.factor();
    let c = this.peek();
    while (c === "*" || c === "/") {
      this.pos = this.pos + 1;
      const right = this.factor();
      left = { kind: "bin", op: c, left: left, right: right };
      c = this.peek();
    }
    return left;
  }

  factor(): Expr {
    const c = this.peek();
    if (c === "-") {
      this.pos = this.pos + 1;
      return { kind: "neg", arg: this.factor() };
    }
    if (c === "(") {
      this.pos = this.pos + 1;
      const e = this.expr();
      this.expect(")");
      return e;
    }
    if (isDigit(c) || c === ".") {
      return this.number();
    }
    if (isLetter(c)) {
      const start = this.pos;
      while (isLetter(this.text.charAt(this.pos))) {
        this.pos = this.pos + 1;
      }
      const word = this.text.slice(start, this.pos);
      if (this.peek() === "(") {
        return this.call(word.toLowerCase());
      }
      this.pos = start;
      return this.ref();
    }
    return this.fail(c === "" ? "the formula ends too soon" : `unexpected "${c}"`);
  }

  number(): Expr {
    const start = this.pos;
    while (isDigit(this.text.charAt(this.pos)) || this.text.charAt(this.pos) === ".") {
      this.pos = this.pos + 1;
    }
    const n = parseNumber(this.text.slice(start, this.pos));
    return n === n ? { kind: "num", value: n } : this.fail("a bad number");
  }

  /** A cell reference: one letter and a row number, 0 to 99. -1 for an error. */
  refCell(): number {
    const col = LETTERS.indexOf(this.peek().toUpperCase());
    if (col < 0) {
      this.fail("expected a cell name");
      return -1;
    }
    this.pos = this.pos + 1;
    const start = this.pos;
    while (isDigit(this.text.charAt(this.pos))) {
      this.pos = this.pos + 1;
    }
    const row = parseNumber(this.text.slice(start, this.pos));
    if (this.pos === start || row >= ROWS) {
      this.fail("a cell is A0 to Z99");
      return -1;
    }
    return row * COLS + col;
  }

  ref(): Expr {
    const cell = this.refCell();
    if (cell < 0) {
      return { kind: "error", message: this.error };
    }
    return { kind: "ref", cell: cell };
  }

  call(name: string): Expr {
    this.expect("(");
    const args: Expr[] = [];
    if (this.peek() !== ")") {
      args.push(this.arg());
      while (this.peek() === ",") {
        this.pos = this.pos + 1;
        args.push(this.arg());
      }
    }
    this.expect(")");
    return { kind: "call", name: name, args: args };
  }

  /** A range (A1:B3) or an expression. */
  arg(): Expr {
    const start = this.pos;
    if (isLetter(this.peek())) {
      const from = this.refCell();
      if (from >= 0 && this.peek() === ":") {
        this.pos = this.pos + 1;
        const to = this.refCell();
        if (to < 0) {
          return { kind: "error", message: this.error };
        }
        const cells: number[] = [];
        const c1 = Math.min(from % COLS, to % COLS);
        const c2 = Math.max(from % COLS, to % COLS);
        const r1 = Math.min(Math.floor(from / COLS), Math.floor(to / COLS));
        const r2 = Math.max(Math.floor(from / COLS), Math.floor(to / COLS));
        for (let r = r1; r <= r2; r++) {
          for (let c = c1; c <= c2; c++) {
            cells.push(r * COLS + c);
          }
        }
        return { kind: "range", cells: cells };
      }
      this.pos = start;
      this.error = "";
    }
    return this.expr();
  }
}

/** Parses the content of a cell. */
export function parse(content: string): Expr {
  const text = content.trim();
  if (text === "") {
    return { kind: "empty" };
  }
  if (!text.startsWith("=")) {
    const n = parseNumber(text);
    if (n === n) {
      return { kind: "num", value: n };
    }
    return { kind: "text", text: text };
  }
  const p = new Parser(text.slice(1));
  const e = p.expr();
  if (p.error === "" && p.peek() !== "") {
    p.fail(`unexpected "${p.peek()}"`);
  }
  if (p.error !== "") {
    return { kind: "error", message: p.error };
  }
  return e;
}

/** Adds the cells that an expression reads to `out`. */
export function refs(e: Expr, out: number[]): void {
  if (e.kind === "ref") {
    out.push(e.cell);
  } else if (e.kind === "range") {
    for (const c of e.cells) {
      out.push(c);
    }
  } else if (e.kind === "call") {
    for (const a of e.args) {
      refs(a, out);
    }
  } else if (e.kind === "bin") {
    refs(e.left, out);
    refs(e.right, out);
  } else if (e.kind === "neg") {
    refs(e.arg, out);
  }
}

export function error(message: string): CellValue {
  return { kind: "error", num: 0, text: message };
}

export function num(n: number): CellValue {
  if (n !== n || n === 1 / 0 || n === -1 / 0) {
    return error("not a number");
  }
  return { kind: "num", num: n, text: "" };
}

/** A number for arithmetic: an empty cell is 0; text is not a number (-1 marks it in `ok`). */
function asNumber(v: CellValue): number | null {
  if (v.kind === "num") {
    return v.num;
  }
  if (v.kind === "empty") {
    return 0;
  }
  return null;
}

/** Evaluates `e`; `cell(i)` gives the value of another cell. */
export function evaluate(e: Expr, cell: (i: number) => CellValue): CellValue {
  if (e.kind === "num") {
    return num(e.value);
  }
  if (e.kind === "text") {
    return { kind: "text", num: 0, text: e.text };
  }
  if (e.kind === "empty") {
    return { kind: "empty", num: 0, text: "" };
  }
  if (e.kind === "error") {
    return error(e.message);
  }
  if (e.kind === "ref") {
    return cell(e.cell);
  }
  if (e.kind === "range") {
    return error("a range is only for a function");
  }
  if (e.kind === "neg") {
    const v = evaluate(e.arg, cell);
    if (v.kind === "error") {
      return v;
    }
    const n = asNumber(v);
    if (n === null) {
      return error("not a number");
    }
    return num(-n);
  }
  if (e.kind === "bin") {
    const a = evaluate(e.left, cell);
    if (a.kind === "error") {
      return a;
    }
    const b = evaluate(e.right, cell);
    if (b.kind === "error") {
      return b;
    }
    const x = asNumber(a);
    const y = asNumber(b);
    if (x === null || y === null) {
      return error("not a number");
    }
    if (e.op === "+") {
      return num(x + y);
    }
    if (e.op === "-") {
      return num(x - y);
    }
    if (e.op === "*") {
      return num(x * y);
    }
    if (y === 0) {
      return error("division by zero");
    }
    return num(x / y);
  }
  return call(e.name, e.args, cell);
}

function call(name: string, args: Expr[], cell: (i: number) => CellValue): CellValue {
  // The numbers of all arguments; a range gives one number for each cell
  // that has one (empty and text cells in a range are left out).
  const nums: number[] = [];
  for (const a of args) {
    if (a.kind === "range") {
      for (const c of a.cells) {
        const v = cell(c);
        if (v.kind === "error") {
          return v;
        }
        if (v.kind === "num") {
          nums.push(v.num);
        }
      }
    } else {
      const v = evaluate(a, cell);
      if (v.kind === "error") {
        return v;
      }
      const n = asNumber(v);
      if (n === null) {
        return error("not a number");
      }
      nums.push(n);
    }
  }
  if (name === "sum" || name === "add") {
    let t = 0;
    for (const n of nums) {
      t = t + n;
    }
    return num(t);
  }
  if (name === "prod" || name === "mul") {
    let t = 1;
    for (const n of nums) {
      t = t * n;
    }
    return num(t);
  }
  if (name === "avg") {
    if (nums.length === 0) {
      return error("no numbers");
    }
    let t = 0;
    for (const n of nums) {
      t = t + n;
    }
    return num(t / nums.length);
  }
  if (name === "min" || name === "max") {
    if (nums.length === 0) {
      return error("no numbers");
    }
    let t = nums[0];
    for (const n of nums) {
      t = name === "min" ? Math.min(t, n) : Math.max(t, n);
    }
    return num(t);
  }
  if (name === "sub" || name === "div") {
    if (nums.length !== 2) {
      return error(`${name} takes two numbers`);
    }
    if (name === "sub") {
      return num(nums[0] - nums[1]);
    }
    if (nums[1] === 0) {
      return error("division by zero");
    }
    return num(nums[0] / nums[1]);
  }
  return error(`no function "${name}"`);
}
