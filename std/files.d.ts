// plinth:files: text files in the app's private space (core 1.11,
// docs/STORAGE.md §2, §3; docs/host-apis.md "plinth:files"). Needs the
// `files.private` capability (Low risk: granted with no question, like
// `store.kv`). The host keeps the files: a folder for each app on the
// desktop, the page's IndexedDB on the web. No other app can read them.
//
// A path is relative, with `/` between segments, for example
// "notes/today.md". The host refuses absolute paths, `.` and `..`, empty
// segments, `\`, `:`, control characters and a few other names
// (docs/host-apis.md lists the rules). `""` is the root (for `list` only).
//
// Each call returns at once. Without `done`, it returns a `Promise` that
// rejects with an `Error` when the call fails; the message is the reason:
// "denied:undeclared", "denied:refused", "denied:unsupported",
// "invalid-path: <rule>", "not-found", "not-a-file", "not-a-directory",
// "not-text", "too-large", "quota" or "io: <detail>". With `done`, the
// first argument is that reason, or `null` on success. A failed call never
// traps the app.

/** One entry of a directory. */
export interface FileEntry {
  /** The last segment of the path. */
  name: string;
  /** `"file"` or `"dir"`. */
  kind: string;
  /** The size of a file in bytes (UTF-8); `0` for a directory. */
  size: number;
}

/** The text of the file at `path`. */
export declare function read(path: string): Promise<string>;
export declare function read(path: string, done: (error: string | null, text: string) => void): void;

/** Writes `text` to `path` (replaces the file; makes the parent directories). */
export declare function write(path: string, text: string): Promise<void>;
export declare function write(path: string, text: string, done: (error: string | null) => void): void;

/** The entries of the directory `dir`, sorted by name (`""` is the root). */
export declare function list(dir: string): Promise<FileEntry[]>;
export declare function list(dir: string, done: (error: string | null, entries: FileEntry[]) => void): void;

/** The entry at `path`, or `null` if nothing is there. */
export declare function stat(path: string): Promise<FileEntry | null>;
export declare function stat(path: string, done: (error: string | null, entry: FileEntry | null) => void): void;

/** Removes the file or the directory (with its contents) at `path`. A path
 * that does not exist is not an error. */
export declare function remove(path: string): Promise<void>;
export declare function remove(path: string, done: (error: string | null) => void): void;
