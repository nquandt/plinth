// plinth:core: numbers, math, strings and dev logging (SPEC.md §4.7).

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
