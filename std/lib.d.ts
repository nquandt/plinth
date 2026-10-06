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
  readonly size: number;
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
}

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
interface Iterator<T> {
  next(): IteratorResult<T>;
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
