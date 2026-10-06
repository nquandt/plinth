// The app state. Screens import these signals and functions.
import { signal, computed } from "plinth:ui";

export interface Contact {
  id: number;
  name: string;
  phone: string;
  favorite: boolean;
}

let nextId = 1;

function makeContact(name: string, phone: string, favorite: boolean): Contact {
  const contact: Contact = { id: nextId, name, phone, favorite };
  nextId += 1;
  return contact;
}

export const contacts = signal<Contact[]>([
  makeContact("Ada Lovelace", "555-0101", true),
  makeContact("Grace Hopper", "555-0102", true),
  makeContact("Alan Turing", "555-0103", false),
]);

/** "all" or "favorites": the selected `Tabs` value on the list screen. */
export const tab = signal("all");

/** The contact the detail screen is open on. `null` means none. */
export const selected = signal<number | null>(null);

export const visibleContacts = computed(() => {
  if (tab() === "favorites") {
    return contacts().filter((c) => c.favorite);
  }
  return contacts();
});

export function selectedContact(): Contact | null {
  const id = selected();
  if (id === null) {
    return null;
  }
  const found = contacts().find((c) => c.id === id);
  if (found === undefined) {
    return null;
  }
  return found;
}

export function addContact(name: string, phone: string): boolean {
  const trimmed = name.trim();
  if (trimmed === "") {
    return false;
  }
  contacts.set([...contacts(), makeContact(trimmed, phone, false)]);
  return true;
}

export function updateContact(id: number, name: string, phone: string): void {
  contacts.set(contacts().map((c) => (c.id === id ? { ...c, name, phone } : c)));
}

export function toggleFavorite(id: number): void {
  contacts.set(contacts().map((c) => (c.id === id ? { ...c, favorite: !c.favorite } : c)));
}

export function deleteContact(id: number): void {
  contacts.set(contacts().filter((c) => c.id !== id));
  if (selected() === id) {
    selected.set(null);
  }
}

export function selectContact(id: number): void {
  selected.set(id);
}
