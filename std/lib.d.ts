// The global types of Plinth TS. The project tsconfig sets "noLib", so these
// declarations replace the ECMAScript library. Only the members here exist;
// the Plinth compiler rejects everything else.

interface Array<T> {
  readonly length: number;
  [index: number]: T;
  push(...items: T[]): number;
  pop(): T | null;
  map<U>(f: (value: T, index: number) => U): U[];
  filter(f: (value: T, index: number) => boolean): T[];
  find(f: (value: T) => boolean): T | null;
  findIndex(f: (value: T) => boolean): number;
  some(f: (value: T) => boolean): boolean;
  every(f: (value: T) => boolean): boolean;
  forEach(f: (value: T, index: number) => void): void;
  includes(value: T): boolean;
  indexOf(value: T): number;
  slice(start?: number, end?: number): T[];
  reverse(): T[];
  join(separator?: string): string;
  concat(...items: T[][]): T[];
  reduce<U>(f: (acc: U, value: T, index: number) => U, initial: U): U;
  sort(compare?: (a: T, b: T) => number): T[];
  splice(start: number, deleteCount?: number, ...items: T[]): T[];
  fill(value: T, start?: number, end?: number): T[];
  /** One level only: `depth` can only be 1. */
  flat<U>(this: U[][], depth?: 1): U[];
  flat(depth?: 1): T[];
  [Symbol.iterator](): Iterator<T>;
}

interface ReadonlyArray<T> {
  readonly length: number;
  readonly [index: number]: T;
}

interface String {
  readonly length: number;
  trim(): string;
  toUpperCase(): string;
  toLowerCase(): string;
  includes(search: string): boolean;
  startsWith(search: string): boolean;
  endsWith(search: string): boolean;
  indexOf(search: string): number;
  slice(start?: number, end?: number): string;
  repeat(count: number): string;
  lastIndexOf(search: string): number;
  replace(search: string, replacement: string): string;
  replaceAll(search: string, replacement: string): string;
  padStart(targetLength: number, pad?: string): string;
  padEnd(targetLength: number, pad?: string): string;
  charAt(index: number): string;
  split(separator: string): string[];
}

interface Number {
  toFixed(digits?: number): string;
}

/**
 * A key must be `string`, `int`, `number`, `boolean` or an enum
 * (SPEC.md §4.2). `get` returns `null`, not `undefined`, when the key is
 * absent (§4.6). Lookup, insert and delete are linear scans.
 */
declare class Map<K, V> {
  constructor();
  get(key: K): V | null;
  set(key: K, value: V): void;
  has(key: K): boolean;
  delete(key: K): boolean;
  clear(): void;
  forEach(f: (value: V, key: K) => void): void;
  keys(): K[];
  values(): V[];
  /** A new array of `[key, value]` pairs, in insertion order. */
  entries(): [K, V][];
  readonly size: number;
  /** `for (const [k, v] of m)`: the entries in insertion order. */
  [Symbol.iterator](): Iterator<[K, V]>;
}

/** An element must be `string`, `int`, `number`, `boolean` or an enum. */
declare class Set<T> {
  constructor();
  add(value: T): void;
  has(value: T): boolean;
  delete(value: T): boolean;
  clear(): void;
  forEach(f: (value: T) => void): void;
  keys(): T[];
  values(): T[];
  readonly size: number;
  /** `for (const v of s)`: the elements in insertion order. */
  [Symbol.iterator](): Iterator<T>;
}

/**
 * The value that `throw` takes and that a `catch` variable gets (SPEC.md
 * §5.6). `throw "text"` throws `new Error("text")`. Extend it for your own
 * errors, and narrow a caught error with `instanceof`.
 */
declare class Error {
  constructor(message?: string);
  name: string;
  message: string;
}

/**
 * The result of an `async` function, or of a host call without a `done`
 * callback (SPEC.md §4.5). Read the value with `await` inside an `async`
 * function, or with `then`. A callback may return a value or a promise.
 * A rejection reason is always an `Error`.
 */
interface Promise<T> {
  then<R = T>(onfulfilled: (value: T) => R | PromiseLike<R>, onrejected?: (reason: Error) => R | PromiseLike<R>): Promise<R>;
  /** `onrejected` returns a value of the same type `T` (or a promise of it). */
  catch(onrejected: (reason: Error) => T | PromiseLike<T>): Promise<T>;
  finally(onfinally: () => void | PromiseLike<void>): Promise<T>;
}
// `tsc` needs this for `async` functions.
interface PromiseLike<T> {
  then<R = T>(onfulfilled: (value: T) => R | PromiseLike<R>, onrejected?: (reason: Error) => R | PromiseLike<R>): PromiseLike<R>;
}
interface PromiseConstructor {
  readonly prototype: Promise<unknown>;
  /** Write the type: `new Promise<T>((resolve, reject) => { ... })`. */
  new <T>(executor: (resolve: (value: T) => void, reject: (reason: Error) => void) => void): Promise<T>;
  /** Waits for every promise; rejects with the first rejection. */
  all<T>(values: Promise<T>[]): Promise<T[]>;
  /** An array literal of promises of different types: a tuple. */
  all<A, B>(values: [Promise<A>, Promise<B>]): Promise<[A, B]>;
  all<A, B, C>(values: [Promise<A>, Promise<B>, Promise<C>]): Promise<[A, B, C]>;
  all<A, B, C, D>(values: [Promise<A>, Promise<B>, Promise<C>, Promise<D>]): Promise<[A, B, C, D]>;
  /** Settles as the first promise that settles. */
  race<T>(values: Promise<T>[]): Promise<T>;
  /**
   * Resolves with the first value. When all promises reject, rejects with
   * an `Error` whose `name` is "AggregateError".
   */
  any<T>(values: Promise<T>[]): Promise<T>;
  /** Waits until all promises settle; one result per promise, in order. */
  allSettled<T>(values: Promise<T>[]): Promise<PromiseSettledResult<T>[]>;
  resolve(): Promise<void>;
  resolve<T>(value: T | Promise<T>): Promise<T>;
  /** `reason` is an `Error`, or a string that becomes `new Error(reason)`. */
  reject<T = void>(reason: Error | string): Promise<T>;
}
declare var Promise: PromiseConstructor;
interface PromiseFulfilledResult<T> {
  status: "fulfilled";
  value: T;
}
interface PromiseRejectedResult {
  status: "rejected";
  reason: Error;
}
/** An element of the result of `Promise.allSettled`. Narrow it with `status`. */
type PromiseSettledResult<T> = PromiseFulfilledResult<T> | PromiseRejectedResult;

interface Boolean {}
interface Function {}
interface CallableFunction extends Function {}
interface NewableFunction extends Function {}
interface Object {}
interface RegExp {}
interface IArguments {}
interface TemplateStringsArray extends ReadonlyArray<string> {}

interface SymbolConstructor {
  readonly iterator: unique symbol;
}
declare var Symbol: SymbolConstructor;
interface IteratorResult<T> {
  done?: boolean;
  value: T;
}
// `tsc` accepts `for…of` over a type other than an array (`Map`, `Set`) only
// when the global `Iterator` and `Iterable` types exist with these three
// type parameters (the ES2015 library shape). Plinth uses only `T`.
interface Iterator<T, TReturn = unknown, TNext = unknown> {
  next(): IteratorResult<T>;
}
interface Iterable<T, TReturn = unknown, TNext = unknown> {
  [Symbol.iterator](): Iterator<T, TReturn, TNext>;
}

declare namespace JSX {
  // An element is a node of the semantic tree.
  interface Element {
    readonly __plinthElement: true;
  }
  interface ElementChildrenAttribute {
    children: {};
  }
  interface IntrinsicElements {}
}
