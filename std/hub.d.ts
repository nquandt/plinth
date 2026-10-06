// plinth:hub: privileged library and Hub management calls (`docs/HUB.md`
// §4.1, §12.2). Every call needs the `hub.manage` capability, and the host
// grants it only to a package signed by a key it trusts as a Hub key
// (`docs/HUB.md` §4.1); every other app that imports this module is
// refused at load, with a clear error, before it ever runs.

/** Every app in the library (`docs/HUB.md` §9.1), as a JSON string:
 * `Array<{ id: string, name: string, version: string, signer: string,
 * blocked: boolean, capabilities: Array<{ name: string, risk: "none" |
 * "low" | "medium" | "high", allowed: boolean }> }>`. `signer` is `""` for
 * an unsigned app. Decode it with `JSON.parse` (`plinth:core`, SPEC.md
 * §4.7). Returns `null` if denied. */
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

/** The reason the last `plinth:hub` call was denied, one of
 * `"denied:undeclared"`, `"denied:refused"`, `"denied:unsupported"`, or
 * `null` if the last call was not denied. */
export declare function lastError(): string | null;
