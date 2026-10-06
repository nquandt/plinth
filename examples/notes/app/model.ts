// The app state. Screens import these signals and functions.
import { signal, computed, effect } from "plinth:ui";
import { loadNotes, saveNotes } from "./storage";
import type { Note } from "./types";

let nextId = 1;

function makeNote(title: string, body: string): Note {
  const note: Note = { id: nextId, title, body };
  nextId += 1;
  return note;
}

function defaultNotes(): Note[] {
  return [
    makeNote("Welcome", "This is a notes app written in Plinth TS."),
    makeNote("Shopping list", "Milk, eggs, bread, coffee."),
    makeNote("Idea", "A calculator example app would be a good companion."),
  ];
}

// Saved notes (SPEC.md §8.5, `store.kv`) take over `nextId` so new notes
// do not collide with saved ids.
function startingNotes(): Note[] {
  const saved = loadNotes();
  if (saved === null) {
    return defaultNotes();
  }
  for (const n of saved) {
    if (n.id >= nextId) {
      nextId = n.id + 1;
    }
  }
  return saved;
}

export const notes = signal<Note[]>(startingNotes());

// Persists every change. `effect` reruns whenever `notes()` changes.
effect(() => {
  saveNotes(notes());
});

export const query = signal("");

export const selected = signal<number | null>(null);

function bodyPreview(body: string): string {
  if (body.length <= 40) {
    return body;
  }
  return `${body.slice(0, 40)}...`;
}

export const filteredNotes = computed(() => {
  const q = query().trim().toLowerCase();
  if (q === "") {
    return notes();
  }
  return notes().filter((n) => n.title.toLowerCase().includes(q) || n.body.toLowerCase().includes(q));
});

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
  const found = notes().find((n) => n.id === id);
  if (found === undefined) {
    return null;
  }
  return found;
}

/** Adds a note. Returns false when the title is empty. */
export function addNote(title: string, body: string): boolean {
  const trimmed = title.trim();
  if (trimmed === "") {
    return false;
  }
  notes.set([...notes(), makeNote(trimmed, body)]);
  return true;
}

export function updateNote(id: number, title: string, body: string): void {
  notes.set(notes().map((n) => (n.id === id ? { ...n, title, body } : n)));
}

export function deleteNote(id: number): void {
  notes.set(notes().filter((n) => n.id !== id));
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
