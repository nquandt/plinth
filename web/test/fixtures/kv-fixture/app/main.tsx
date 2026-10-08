// A fixture app for web/test/run-hub-storage.mjs: notes kept as one JSON
// value in `store.kv`, as `examples/notes` did before it used `store.sql`.
// Each change writes the whole list.
import { app, signal, effect, Screen, Section, TextField, Button, List, Row } from "plinth:ui";
import { kv } from "plinth:store";
import { JSON } from "plinth:core";

interface Note {
  id: number;
  title: string;
  body: string;
}

function load(): Note[] {
  const raw = kv.get("notes");
  if (raw === null) {
    return [];
  }
  const parsed = JSON.parse<Note[]>(raw);
  return parsed === null ? [] : parsed;
}

const saved = load();
const notes = signal<Note[]>(saved);
let nextId = saved.length + 1;

effect(() => {
  kv.set("notes", JSON.stringify(notes()));
});

function Home() {
  const title = signal("");
  const body = signal("");
  const add = () => {
    if (title().trim() === "") {
      return;
    }
    notes.set([...notes(), { id: nextId, title: title(), body: body() }]);
    nextId += 1;
    title.set("");
    body.set("");
  };
  return (
    <Screen title="KV Fixture">
      <Section title="Add note">
        <TextField label="Title" value={title} />
        <TextField label="Body" value={body} />
        <Button label="Add note" onPress={add} />
      </Section>
      <Section title="All notes">
        <List items={notes()} key={(n) => n.id} row={(n) => <Row title={n.title} subtitle={n.body} />} />
      </Section>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Home", component: Home } } });
