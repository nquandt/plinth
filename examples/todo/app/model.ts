// The app state. Screens import these signals and functions.
import { signal, computed } from "plinth:ui";

export interface Task {
  id: number;
  title: string;
  done: boolean;
}

let nextId = 1;

function makeTask(title: string): Task {
  const task: Task = { id: nextId, title, done: false };
  nextId += 1;
  return task;
}

export const tasks = signal<Task[]>([
  makeTask("Read SPEC.md"),
  makeTask("Build the M0 host"),
  makeTask("Write the compiler"),
]);

export const showCompleted = signal(true);

export const visibleTasks = computed(() => tasks().filter((t) => showCompleted() || !t.done));

export const remaining = computed(() => tasks().filter((t) => !t.done).length);

export const completed = computed(() => tasks().length - remaining());

/** Adds a task. Returns false when the title is empty. */
export function addTask(title: string): boolean {
  const trimmed = title.trim();
  if (trimmed === "") {
    return false;
  }
  tasks.set([...tasks(), makeTask(trimmed)]);
  return true;
}

export function setDone(id: number, done: boolean): void {
  tasks.set(tasks().map((t) => (t.id === id ? { ...t, done } : t)));
}

export function deleteTask(id: number): void {
  tasks.set(tasks().filter((t) => t.id !== id));
}

export function clearCompleted(): void {
  tasks.set(tasks().filter((t) => !t.done));
}
