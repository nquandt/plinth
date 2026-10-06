// The model: a list of people and the selected person. The view
// (main.tsx) reads it and calls these functions; it has no list logic.
import { signal, computed } from "plinth:ui";

export interface Person {
  id: number;
  name: string;
  surname: string;
}

let nextId = 4;

export const people = signal<Person[]>([
  { id: 1, name: "Hans", surname: "Emil" },
  { id: 2, name: "Max", surname: "Mustermann" },
  { id: 3, name: "Roman", surname: "Tisch" },
]);

/** The surname prefix that filters the list. */
export const prefix = signal("");

/** The id of the selected person, or 0 for no selection. */
export const selectedId = signal<number>(0);

export const visible = computed(() => {
  const p = prefix().trim().toLowerCase();
  return people().filter((x) => x.surname.toLowerCase().startsWith(p));
});

/** The selected person, if the filter shows it. */
export const selected = computed(() => {
  const id = selectedId();
  return visible().find((x) => x.id === id) ?? null;
});

export function fullName(p: Person): string {
  return `${p.surname}, ${p.name}`;
}

export function create(name: string, surname: string): boolean {
  if (name.trim() === "" && surname.trim() === "") {
    return false;
  }
  const person: Person = { id: nextId, name: name.trim(), surname: surname.trim() };
  nextId = nextId + 1;
  people.set([...people(), person]);
  selectedId.set(person.id);
  return true;
}

export function updateSelected(name: string, surname: string): void {
  const s = selected();
  if (s === null) {
    return;
  }
  people.set(people().map((x) => (x.id === s.id ? { id: x.id, name: name.trim(), surname: surname.trim() } : x)));
}

export function deleteSelected(): void {
  const s = selected();
  if (s === null) {
    return;
  }
  people.set(people().filter((x) => x.id !== s.id));
  selectedId.set(0);
}
