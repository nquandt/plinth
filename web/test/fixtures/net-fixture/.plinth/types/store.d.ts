// plinth:store: a per-app key/value store (SPEC.md §8.5). Needs the
// `store.kv` capability, declared in plinth.toml.

export declare const kv: {
  /** Returns the value for `key`, or `null` if it is not set. */
  get(key: string): string | null;
  /** Sets `key` to `value`, replacing any existing value. */
  set(key: string, value: string): void;
  /** Removes `key`. Removing a missing key is not an error. */
  remove(key: string): void;
  /** Lists every key currently set, in no particular order. */
  keys(): string[];
  /** The reason the last `kv` call was denied (SPEC.md §8.5): one of
   * `"denied:undeclared"`, `"denied:refused"`, `"denied:unsupported"`, or
   * `null` if the last call was not denied. Cleared to `null` on the next
   * successful `kv` call. A denied call never traps: `get` returns `null`,
   * `set`/`remove` do nothing, and `keys` returns `[]`. */
  lastError(): string | null;
};
