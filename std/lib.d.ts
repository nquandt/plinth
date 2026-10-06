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
}

interface Number {
  toFixed(digits?: number): string;
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
