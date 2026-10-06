// plinth:clipboard: the system clipboard (SPEC.md §8.5). `writeText` needs
// the `clipboard.write` capability; `readText` needs `clipboard.read`.

/** Replaces the clipboard's text content. */
export declare function writeText(text: string): void;

/** Reads the clipboard's text content, or `null` if it holds no text
 * (for example an image). */
export declare function readText(): string | null;

/** The reason the last clipboard call was denied (SPEC.md §8.5): one of
 * `"denied:undeclared"`, `"denied:refused"`, `"denied:unsupported"`, or
 * `null` if the last call was not denied. Cleared to `null` on the next
 * successful clipboard call. A denied call never traps: `readText` returns
 * `null` and `writeText` does nothing. */
export declare function lastError(): string | null;
