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
  /** A number in [0, 1) (core 1.12). The host seeds it with fresh entropy
   * for each run; `seedRandom` makes the numbers repeat. Not for security. */
  random(): number;
};

/** Seeds `Math.random` (core 1.12): the same seed gives the same numbers,
 * for a replay or a daily puzzle. */
export declare function seedRandom(seed: number): void;

/** Converts a string to a number, as the JS `Number(s)` function does. */
export declare function parseNumber(s: string): number;

/** Converts a value to its string form. */
export declare function toString(value: number | boolean | string): string;

/**
 * JSON encoding (SPEC.md §4.7). `stringify` serializes numbers, `int`,
 * booleans, strings, `null`/nullable values, arrays, objects of a known
 * shape (in declaration order) and `Map<string, V>` (as an object);
 * `NaN`/`Infinity` become `null`, as in JS. `parse<T>` decodes the same
 * shapes back, given an explicit type argument (write
 * `JSON.parse<YourType>(text)`); it returns `null` on invalid JSON or a
 * shape mismatch (a missing field or a field of the wrong type). Extra
 * object fields are ignored, and `null` is fine for a nullable field.
 */
export declare const JSON: {
  stringify(value: unknown): string;
  parse<T>(text: string): T | null;
};

/** Development logging. The host shows the messages in its log. */
export declare const console: {
  log(message: string): void;
};
