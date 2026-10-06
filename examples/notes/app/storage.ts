// Persists notes to the per-app key/value store (SPEC.md §8.5). Needs the
// `store.kv` capability, declared in plinth.toml.
import { kv } from "plinth:store";
import { parseNumber, toString } from "plinth:core";
import type { Note } from "./types";

const KEY = "notes";
// Control characters, unlikely to appear in typed note text. A real app
// would escape them; this example keeps the encoding simple.
const FIELD_SEP = "\u0001";
const RECORD_SEP = "\u0002";

function encodeNotes(notes: Note[]): string {
  const records: string[] = [];
  for (const n of notes) {
    records.push(toString(n.id) + FIELD_SEP + n.title + FIELD_SEP + n.body);
  }
  return records.join(RECORD_SEP);
}

function decodeNotes(raw: string): Note[] {
  if (raw === "") {
    return [];
  }
  const result: Note[] = [];
  let rest = raw;
  while (true) {
    const recEnd = rest.indexOf(RECORD_SEP);
    const record = recEnd === -1 ? rest : rest.slice(0, recEnd);
    const firstSep = record.indexOf(FIELD_SEP);
    const idStr = record.slice(0, firstSep);
    const afterId = record.slice(firstSep + 1);
    const secondSep = afterId.indexOf(FIELD_SEP);
    const title = afterId.slice(0, secondSep);
    const body = afterId.slice(secondSep + 1);
    result.push({ id: parseNumber(idStr), title, body });
    if (recEnd === -1) {
      break;
    }
    rest = rest.slice(recEnd + 1);
  }
  return result;
}

/** Writes `notes` to the store, replacing whatever was saved before. */
export function saveNotes(notes: Note[]): void {
  kv.set(KEY, encodeNotes(notes));
}

/** Reads the saved notes, or `null` if nothing was saved yet. */
export function loadNotes(): Note[] | null {
  const raw = kv.get(KEY);
  if (raw === null) {
    return null;
  }
  return decodeNotes(raw);
}
