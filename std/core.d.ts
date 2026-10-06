// plinth:core: numbers, math, strings and dev logging (SPEC.md §4.7).

/**
 * A 32-bit integer (SPEC.md §4.2): a branded `number`. Literals and
 * arithmetic on `int` stay `int`; it converts to `number` implicitly.
 * Convert a `number` to `int` with `int(x)` (it truncates).
 */
export type int = number & { readonly __int: unique symbol };
export declare function int(x: number): int;

/** Mathematical functions. */
export declare const Math: {
  readonly PI: number;
  readonly E: number;
  floor(x: number): number;
  ceil(x: number): number;
  round(x: number): number;
  trunc(x: number): number;
  abs(x: number): number;
  sqrt(x: number): number;
  min(a: number, b: number): number;
  max(a: number, b: number): number;
  pow(x: number, y: number): number;
};

/** Converts a string to a number, as the JS `Number(s)` function does. */
export declare function parseNumber(s: string): number;

/** Converts a value to its string form. */
export declare function toString(value: number | boolean | string): string;

/** Development logging. The host shows the messages in its log. */
export declare const console: {
  log(message: string): void;
};
