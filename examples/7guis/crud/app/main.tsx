// 7GUIs task 5, CRUD (https://eugenkiss.github.io/7guis/tasks#crud).
//
// A list of names with a surname filter. Select a row to edit it. "Create"
// adds the name in the fields, "Update" changes the selected entry, and
// "Delete" removes it. Update and Delete need a selected entry.
import { app, signal, Screen, Section, TextField, List, Row, Empty, Button, Group } from "plinth:ui";
import { prefix, visible, selected, selectedId, fullName, create, updateSelected, deleteSelected } from "./model";

function Crud() {
  const name = signal("");
  const surname = signal("");

  const select = (id: number, first: string, last: string) => {
    selectedId.set(id);
    name.set(first);
    surname.set(last);
  };

  return (
    <Screen title="CRUD">
      <Section>
        <TextField label="Filter prefix" placeholder="Surname starts with" value={prefix} />
        <List
          items={visible()}
          key={(p) => p.id}
          row={(p) => (
            <Row
              title={fullName(p)}
              selected={selectedId() === p.id}
              onPress={() => select(p.id, p.name, p.surname)}
            />
          )}
          empty={<Empty title="No entries" message="No surname starts with this prefix." />}
        />
      </Section>
      <Section>
        <TextField label="Name" value={name} />
        <TextField label="Surname" value={surname} />
        <Group axis="row">
          <Button label="Create" role="primary" onPress={() => create(name(), surname())} />
          <Button label="Update" disabled={selected() === null} onPress={() => updateSelected(name(), surname())} />
          <Button label="Delete" role="destructive" disabled={selected() === null} onPress={deleteSelected} />
        </Group>
      </Section>
    </Screen>
  );
}

export default app({
  accent: "indigo",
  screens: {
    crud: { title: "CRUD", icon: "list", component: Crud },
  },
});
