// The app state: a signal, plus persistence with the key/value store
// (SPEC.md §8.5). Needs the `store.kv` capability, declared in plinth.toml.
import { signal, computed } from "plinth:ui";
import { kv } from "plinth:store";
import { JSON } from "plinth:core";

export interface Task {
  id: number;
  title: string;
  done: boolean;
}

const KEY = "tasks";

let nextId = 1;

function makeTask(title: string): Task {
  const task: Task = { id: nextId, title, done: false };
  nextId += 1;
  return task;
}

/** Reads the saved tasks, or `[]` if nothing was saved yet (or the saved
 * text does not parse as a `Task[]`). */
function loadTasks(): Task[] {
  const raw = kv.get(KEY);
  if (raw === null) {
    return [];
  }
  const parsed = JSON.parse<Task[]>(raw);
  if (parsed === null) {
    return [];
  }
  return parsed;
}

export const tasks = signal<Task[]>(loadTasks());

/** The task the detail screen is open on. `null` means none. */
export const selected = signal<number | null>(null);

/** How many tasks are not done yet. Recomputed only when `tasks` changes. */
export const remaining = computed(() => tasks().filter((t) => !t.done).length);

function persist(next: Task[]): void {
  tasks.set(next);
  kv.set(KEY, JSON.stringify(next));
}

export function selectedTask(): Task | null {
  const id = selected();
  if (id === null) {
    return null;
  }
  const found = tasks().find((t) => t.id === id);
  if (found === undefined) {
    return null;
  }
  return found;
}

export function addTask(title: string): boolean {
  const trimmed = title.trim();
  if (trimmed === "") {
    return false;
  }
  persist([...tasks(), makeTask(trimmed)]);
  return true;
}

export function toggleDone(id: number): void {
  persist(tasks().map((t) => (t.id === id ? { ...t, done: !t.done } : t)));
}

export function deleteTask(id: number): void {
  persist(tasks().filter((t) => t.id !== id));
  if (selected() === id) {
    selected.set(null);
  }
}

export function selectTask(id: number): void {
  selected.set(id);
}
