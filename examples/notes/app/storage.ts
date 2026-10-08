// Keeps the notes in the app's SQL database (`store.sql`, docs/SQL.md).
// Needs the `store.sql` capability, declared in plinth.toml.
//
// The table `notes` holds the notes. `changed` is a change counter: each
// insert or update sets it to one more than the highest value, so the
// highest value is the last change. The full-text index `notes_fts` (SQLite
// FTS5) reads its text from `notes`; the triggers keep it in step.
//
// Older builds of this example kept all notes as one JSON value in
// `store.kv` (key "notes"). `openDatabase` moves them into the table one
// time, then removes the key.
import { kv, sql } from "plinth:store";
import { JSON } from "plinth:core";
import type { Note } from "./types";

// Migration steps are append-only: never change or remove a step.
const SCHEMA = [
  "CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL, body TEXT NOT NULL, changed INTEGER NOT NULL)",
  "CREATE INDEX notes_changed ON notes (changed)",
  "CREATE VIRTUAL TABLE notes_fts USING fts5(title, body, content='notes', content_rowid='id')",
  "CREATE TRIGGER notes_ai AFTER INSERT ON notes BEGIN INSERT INTO notes_fts (rowid, title, body) VALUES (new.id, new.title, new.body); END",
  "CREATE TRIGGER notes_ad AFTER DELETE ON notes BEGIN INSERT INTO notes_fts (notes_fts, rowid, title, body) VALUES ('delete', old.id, old.title, old.body); END",
  "CREATE TRIGGER notes_au AFTER UPDATE ON notes BEGIN INSERT INTO notes_fts (notes_fts, rowid, title, body) VALUES ('delete', old.id, old.title, old.body); INSERT INTO notes_fts (rowid, title, body) VALUES (new.id, new.title, new.body); END",
];

const INSERT = "INSERT INTO notes (title, body, changed) VALUES (?, ?, (SELECT coalesce(max(changed), 0) + 1 FROM notes))";
const UPDATE = "UPDATE notes SET title = ?, body = ?, changed = (SELECT coalesce(max(changed), 0) + 1 FROM notes) WHERE id = ?";
const DELETE = "DELETE FROM notes WHERE id = ?";
const ALL = "SELECT id, title, body FROM notes ORDER BY changed DESC";
const SEARCH =
  "SELECT n.id, n.title, n.body FROM notes_fts JOIN notes n ON n.id = notes_fts.rowid WHERE notes_fts MATCH ? ORDER BY n.changed DESC";

/** The key of the old JSON value in `store.kv`. */
const OLD_KEY = "notes";

/** The notes of a new database, the last one on top. */
const FIRST_NOTES = [
  ["Idea", "A calculator example app would be a good companion."],
  ["Shopping list", "Milk, eggs, bread, coffee."],
  ["Welcome", "This is a notes app written in Plinth TS."],
];

/** Makes or updates the tables. On the first run, moves the notes of an
 * older build from `store.kv` into the table, or adds the first notes. */
export async function openDatabase(): Promise<void> {
  const before = await sql.query<{ user_version: number }>("PRAGMA user_version");
  await sql.migrate(SCHEMA);
  const counts = await sql.query<{ n: number }>("SELECT count(*) AS n FROM notes");
  const empty = counts[0].n === 0;
  const old = kv.get(OLD_KEY);
  const saved = old === null ? null : JSON.parse<Note[]>(old);
  const steps: { sql: string; params: string[] }[] = [];
  if (saved !== null) {
    // Copy the old notes only into an empty table: if the table has notes,
    // a copy ran before (and the app stopped before it removed the key).
    if (empty) {
      for (const n of saved) {
        steps.push({ sql: INSERT, params: [n.title, n.body] });
      }
    }
  } else if (empty && before[0].user_version === 0) {
    for (const [title, body] of FIRST_NOTES) {
      steps.push({ sql: INSERT, params: [title, body] });
    }
  }
  if (steps.length > 0) {
    await sql.batch(steps);
  }
  if (saved !== null) {
    kv.remove(OLD_KEY);
  }
}

/** All notes, the last change first. */
export function allNotes(): Promise<Note[]> {
  return sql.query<Note>(ALL);
}

/** The notes that have each word of `text` (or a word that starts with
 * it), the last change first. */
export function searchNotes(text: string): Promise<Note[]> {
  const match = matchText(text);
  if (match === "") {
    return allNotes();
  }
  return sql.query<Note>(SEARCH, [match]);
}

/** The FTS5 query for the text of the search box. Each word becomes an FTS5
 * string (a `"` in it is doubled) with `*`, so that the word matches as a
 * prefix ("mil" finds "milk"). The text of a string is never an operator,
 * so a quote, `*`, `-`, `AND` or `NEAR` cannot cause a syntax error. A
 * string with no letters or digits (for example `"-"`) matches nothing
 * alone and is ignored next to other words. */
export function matchText(text: string): string {
  const words: string[] = [];
  for (const word of text.trim().split(/\s+/)) {
    if (word !== "") {
      words.push(`"${word.replaceAll("\"", "\"\"")}"*`);
    }
  }
  return words.join(" ");
}

/** The result of a change: the number of changed rows and the last id. */
export type Changed = Promise<{ changes: number; lastInsertId: number }>;

export function insertNote(title: string, body: string): Changed {
  return sql.exec(INSERT, [title, body]);
}

export function changeNote(id: number, title: string, body: string): Changed {
  return sql.exec(UPDATE, [title, body, id]);
}

export function removeNote(id: number): Changed {
  return sql.exec(DELETE, [id]);
}
