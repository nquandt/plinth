// plinth:clipboard: the system clipboard (SPEC.md §8.5). `writeText` needs
// the `clipboard.write` capability; `readText` needs `clipboard.read`.

/** Replaces the clipboard's text content. */
export declare function writeText(text: string): void;

/** Reads the clipboard's text content, or `null` if it holds no text
 * (for example an image). */
export declare function readText(): string | null;
