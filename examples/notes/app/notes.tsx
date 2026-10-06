import { signal, Screen, Section, TextField, Button, Text, List, Row, Empty } from "plinth:ui";
import {
  notes,
  query,
  filteredNotes,
  selected,
  selectedNote,
  subtitleFor,
  addNote,
  updateNote,
  deleteNote,
  selectNote,
  clearSelection,
} from "./model";

export default function Notes() {
  const draftTitle = signal("");
  const draftBody = signal("");
  const editTitle = signal("");
  const editBody = signal("");

  const submit = () => {
    if (addNote(draftTitle(), draftBody())) {
      draftTitle.set("");
      draftBody.set("");
    }
  };

  const openNote = (id: number) => {
    const note = notes().find((n) => n.id === id);
    if (note === null) {
      return;
    }
    editTitle.set(note.title);
    editBody.set(note.body);
    selectNote(id);
  };

  const saveEdit = () => {
    const note = selectedNote();
    if (note === null) {
      return;
    }
    updateNote(note.id, editTitle(), editBody());
    clearSelection();
  };

  return (
    <Screen title="Notes">
      <Section title="Search">
        <TextField label="Search" placeholder="Filter notes" value={query} />
      </Section>
      {selectedNote() === null ? null : (
        <Section title="Edit note" footer="Changes save when you press Save.">
          <TextField label="Title" value={editTitle} />
          <TextField label="Body" value={editBody} />
          <Button label="Save" role="primary" onPress={saveEdit} />
          <Button label="Cancel" onPress={clearSelection} />
        </Section>
      )}
      <Section title="Add note">
        <TextField label="Title" placeholder="Note title" value={draftTitle} onSubmit={submit} />
        <TextField label="Body" placeholder="Note body" value={draftBody} onSubmit={submit} />
        <Button label="Add note" role="primary" onPress={submit} disabled={draftTitle().trim() === ""} />
      </Section>
      <Section title="All notes">
        <Text tone="muted">{`${filteredNotes().length} of ${notes().length} notes`}</Text>
        <List
          items={filteredNotes()}
          key={(n) => n.id}
          row={(n) => (
            <Row title={n.title} subtitle={subtitleFor(n)} onPress={() => openNote(n.id)}>
              <Button label="Delete" role="destructive" onPress={() => deleteNote(n.id)} />
            </Row>
          )}
          empty={<Empty title={notes().length === 0 ? "No notes yet" : "No matches"} message="Try adding a note above." />}
        />
      </Section>
    </Screen>
  );
}
