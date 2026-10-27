// plinth:secrets: safe storage for passwords and tokens (core 1.13,
// docs/SECRETS.md). Needs the `secrets` capability (Low risk: the store
// belongs to this app only, and the host grants it with no question). The
// host keeps each secret in the safest place that the platform has. Another
// app cannot read it. The host never keeps it in a plain file.
//
// A name has 1 to 64 characters from a-z, A-Z, 0-9, ".", "_" and "-". A
// value is a string of at most 16 KiB. An app has at most 64 secrets.
//
// Each call returns at once with a `Promise`. A denied call rejects with an
// `Error`. The message is "denied:<reason>". The reason is one of:
//   "undeclared"  plinth.toml does not declare the `secrets` capability.
//   "refused"     the user turned the capability off.
//   "unsupported" this host has no safe store. The host does not use a
//                 plain file instead.
//   "limit"       a name, a value or the count of secrets breaks a limit.
//   "failed"      the safe store itself failed (for example a locked
//                 keychain). The message never holds a value.
// A denied call never traps the app.

/** Stores `value` under `name`. A new value replaces the old value. */
export declare function set(name: string, value: string): Promise<void>;

/** Reads the value of `name`. Resolves with `null` when the name does not
 * exist. A missing name is not an error. */
export declare function get(name: string): Promise<string | null>;

/** Removes `name`. A name that does not exist is not an error. */
export declare function remove(name: string): Promise<void>;

/** Lists the names of all secrets of this app, sorted. */
export declare function names(): Promise<string[]>;

/** The reason the last call was denied: for example `"denied:limit"`, or
 * `null` if the last call was not denied. */
export declare function lastError(): string | null;
