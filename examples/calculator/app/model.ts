// Calculator state. The screens import these signals and functions.
import { signal } from "plinth:ui";
import { parseNumber, toString } from "plinth:core";

export type Operator = "+" | "-" | "*" | "/" | null;

export interface Calculation {
  id: number;
  text: string;
}

let nextId = 1;

export const display = signal("0");
export const stored = signal(0);
export const pending = signal<Operator>(null);
export const justEvaluated = signal(false);

export const history = signal<Calculation[]>([]);

function addHistory(text: string): void {
  history.set([{ id: nextId, text }, ...history()]);
  nextId += 1;
}

export function inputDigit(digit: string): void {
  if (justEvaluated()) {
    display.set(digit);
    justEvaluated.set(false);
    return;
  }
  if (display() === "0") {
    display.set(digit);
    return;
  }
  display.set(`${display()}${digit}`);
}

export function inputDot(): void {
  if (justEvaluated()) {
    display.set("0.");
    justEvaluated.set(false);
    return;
  }
  if (display().includes(".")) {
    return;
  }
  display.set(`${display()}.`);
}

export function clear(): void {
  display.set("0");
  stored.set(0);
  pending.set(null);
  justEvaluated.set(false);
}

type BinaryOp = "+" | "-" | "*" | "/";

function apply(a: number, b: number, op: BinaryOp): number | null {
  switch (op) {
    case "+":
      return a + b;
    case "-":
      return a - b;
    case "*":
      return a * b;
    case "/":
      if (b === 0) {
        return null;
      }
      return a / b;
  }
}

export function chooseOperator(op: Operator): void {
  const current = parseNumber(display());
  const op0: BinaryOp | null = pending();
  if (op0 !== null && !justEvaluated()) {
    const result = apply(stored(), current, op0);
    if (result === null) {
      display.set("Error");
      stored.set(0);
      pending.set(null);
      justEvaluated.set(true);
      return;
    }
    stored.set(result);
    display.set(toString(result));
  } else {
    stored.set(current);
  }
  pending.set(op);
  justEvaluated.set(true);
}

export function equals(): void {
  const current = parseNumber(display());
  const op: BinaryOp | null = pending();
  if (op === null) {
    return;
  }
  const result = apply(stored(), current, op);
  if (result === null) {
    addHistory(`${toString(stored())} ${op} ${toString(current)} = Error`);
    display.set("Error");
    stored.set(0);
    pending.set(null);
    justEvaluated.set(true);
    return;
  }
  addHistory(`${toString(stored())} ${op} ${toString(current)} = ${toString(result)}`);
  display.set(toString(result));
  stored.set(result);
  pending.set(null);
  justEvaluated.set(true);
}
