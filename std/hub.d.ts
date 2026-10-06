// plinth:hub: privileged library and Hub management calls (`docs/HUB.md`
// §4.1, §12.2). Every call needs the `hub.manage` capability, and the host
// grants it only to a package signed by a key it trusts as a Hub key
// (`docs/HUB.md` §4.1); every other app that imports this module is
// refused at load, with a clear error, before it ever runs.

/** Every app in the library (`docs/HUB.md` §9.1), as a JSON string:
 * `Array<{ id: string, name: string, version: string, publisher: string,
 * signer: string, source: string, blocked: boolean, groups: string[],
 * capabilities: Array<{ name: string, risk: "none" | "low" | "medium" |
 * "high", description: string, decided: boolean, allowed: boolean,
 * byDefault: boolean }> }>`. `signer` is `""` for an unsigned app;
 * `source` is `""` for an app added from a file. Decode it with
 * `JSON.parse` (`plinth:core`, SPEC.md §4.7). Returns `null` if denied. */
export declare function listApps(): string | null;

/** Opens `id` in a new window, running the normal consent flow
 * (`docs/HUB.md` §4.2, §7.3). Returns at once. A no-op if denied. */
export declare function launch(id: string): void;

/** Changes one capability grant for an installed app (`docs/HUB.md`
 * §7.4). A no-op if denied. */
export declare function setGrant(id: string, capability: string, allowed: boolean): void;

/** Blocks an app: it will not run until `unblock` (`docs/HUB.md` §7.4).
 * A no-op if denied. */
export declare function block(id: string): void;

/** Reverses `block`. A no-op if denied. */
export declare function unblock(id: string): void;

/** The library's groups (`docs/HUB.md` §9.1), as a JSON string array, in
 * the order the user made them. Returns `null` if denied. Core 1.8. */
export declare function listGroups(): string | null;

/** Makes an empty group. A no-op if the group exists or if denied. */
export declare function createGroup(name: string): void;

/** Puts app `id` in `group` (`member` true) or takes it out (`member`
 * false). A no-op if denied. */
export declare function setGroup(id: string, group: string, member: boolean): void;

/** Removes app `id` from the library. Its data and grants stay. A no-op if
 * denied. */
export declare function remove(id: string): void;

/** Searches every configured source (`docs/HUB.md` §5.2) off the UI
 * thread. `done` gets a JSON string `{ hits: Array<{ id: string, name:
 * string, version: string, description: string, source: string }>,
 * errors: string[] }` (one error for each source that failed), or `null`
 * if denied. Core 1.8. */
export declare function search(query: string, done: (json: string | null) => void): void;

/** Installs the latest version of app `id` from the first configured source
 * that lists it, off the UI thread. `done` gets `null` on success, or the
 * error text (`"denied:<reason>"` if denied). Core 1.8. */
export declare function install(id: string, done: (error: string | null) => void): void;

/** The reason the last `plinth:hub` call was denied, one of
 * `"denied:undeclared"`, `"denied:refused"`, `"denied:unsupported"`, or
 * `null` if the last call was not denied. */
export declare function lastError(): string | null;
