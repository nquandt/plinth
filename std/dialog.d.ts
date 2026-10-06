// plinth:dialog: host-owned modal dialogs (SPEC.md §8.5). No capability is
// needed: dialogs are UI, not data access. Each call returns at once; the
// user's answer reaches `done` later, through a `completion` event
// (SPEC.md §8.4). Without `done`, each call returns a `Promise` that an
// `async` function can `await` (SPEC.md §4.5).

/** Shows `message` with an OK button. Calls `done` once the user
 * dismisses it, if given. */
export declare function alert(message: string): Promise<void>;
export declare function alert(message: string, done: () => void): void;

/** Shows `message` with OK and Cancel. Calls `done` with `true` if the
 * user chose OK, `false` for Cancel. */
export declare function confirm(message: string): Promise<boolean>;
export declare function confirm(message: string, done: (ok: boolean) => void): void;

/** Shows `message` with a text field, OK and Cancel. Calls `done` with
 * the entered text, or `null` if the user canceled. */
export declare function prompt(message: string): Promise<string | null>;
export declare function prompt(message: string, done: (value: string | null) => void): void;
