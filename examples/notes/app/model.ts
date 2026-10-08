// The app state. Screens import these signals and functions.
//
// The notes are in the app's SQL database (`storage.ts`). `notes` is a copy
// of the table, the last change first: each change writes to the database
// and then reads the table again. The search box asks the full-text index.
import { signal, computed, effect } from "plinth:ui";
import { openDatabase, allNotes, searchNotes, insertNote, changeNote, removeNote } from "./storage";
import type { Changed } from "./storage";
import type { Note } from "./types";

export const notes = signal<Note[]>([]);

/** A problem with the database, or "" when there is none. */
export const status = signal("Loading notes...");

export const query = signal("");

/** The notes that match `query`, from the last search. */
const found = signal<Note[]>([]);

export const selected = signal<number | null>(null);

// Each search has a number. Only the last search sets `found`, so a slow
// answer to an old search text cannot replace a newer answer.
let lastSearch = 0;

async function search(text: string): Promise<void> {
  lastSearch += 1;
  const mine = lastSearch;
  try {
    const result = await searchNotes(text);
    if (mine === lastSearch) {
      found.set(result);
    }
  } catch (e) {
    status.set(`Search failed: ${(e as Error).message}`);
  }
}

// Searches again when the search text changes.
effect(() => {
  const text = query();
  if (text.trim() !== "") {
    search(text);
  }
});

/** Reads the table again, and the search result if there is a search. */
async function reload(): Promise<void> {
  notes.set(await allNotes());
  const text = query();
  if (text.trim() !== "") {
    await search(text);
  }
}

/** Waits for a change to the database, then shows the new data. */
async function apply(change: Changed): Promise<void> {
  try {
    await change;
    await reload();
  } catch (e) {
    status.set(`The change was not saved: ${(e as Error).message}`);
  }
}

async function start(): Promise<void> {
  try {
    await openDatabase();
    await reload();
    status.set("");
  } catch (e) {
    status.set(`Notes cannot load: ${(e as Error).message}`);
  }
}

start();

function bodyPreview(body: string): string {
  if (body.length <= 40) {
    return body;
  }
  return `${body.slice(0, 40)}...`;
}

export const filteredNotes = computed(() => (query().trim() === "" ? notes() : found()));

export const noteCount = computed(() => notes().length);

export const totalWords = computed(() => {
  let total = 0;
  for (const n of notes()) {
    total += wordCount(n.body);
  }
  return total;
});

function wordCount(text: string): number {
  const trimmed = text.trim();
  if (trimmed === "") {
    return 0;
  }
  let count = 1;
  for (let i = 0; i < trimmed.length; i += 1) {
    if (trimmed.slice(i, i + 1) === " ") {
      count += 1;
    }
  }
  return count;
}

export function subtitleFor(n: Note): string {
  if (n.body.trim() === "") {
    return "Empty note";
  }
  return bodyPreview(n.body);
}

export function selectedNote(): Note | null {
  const id = selected();
  if (id === null) {
    return null;
  }
  const note = notes().find((n) => n.id === id);
  if (note === undefined) {
    return null;
  }
  return note;
}

/** Adds a note. Returns false when the title is empty. */
export function addNote(title: string, body: string): boolean {
  const trimmed = title.trim();
  if (trimmed === "") {
    return false;
  }
  apply(insertNote(trimmed, body));
  return true;
}

export function updateNote(id: number, title: string, body: string): void {
  apply(changeNote(id, title, body));
}

export function deleteNote(id: number): void {
  apply(removeNote(id));
  if (selected() === id) {
    selected.set(null);
  }
}

export function selectNote(id: number): void {
  selected.set(id);
}

export function clearSelection(): void {
  selected.set(null);
}
