// Persists notes to the per-app key/value store (SPEC.md §8.5). Needs the
// `store.kv` capability, declared in plinth.toml.
//
// Stored as a JSON array of `Note` (SPEC.md §4.7), via `JSON.stringify`/
// `JSON.parse<Note[]>`. Older builds of this example stored a
// control-character-separated format instead; that is not readable by
// `loadNotes` below (it returns `null`, same as "nothing saved yet").
import { kv } from "plinth:store";
import { JSON } from "plinth:core";
import type { Note } from "./types";

const KEY = "notes";

/** Writes `notes` to the store, replacing whatever was saved before. */
export function saveNotes(notes: Note[]): void {
  kv.set(KEY, JSON.stringify(notes));
}

/** Reads the saved notes, or `null` if nothing was saved yet (or the
 * saved text does not parse as a `Note[]`). */
export function loadNotes(): Note[] | null {
  const raw = kv.get(KEY);
  if (raw === null) {
    return null;
  }
  return JSON.parse<Note[]>(raw);
}
