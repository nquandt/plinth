import { signal, navigate, Screen, Section, TextField, Button, Text, List, Row, Empty } from "plinth:ui";
import { tasks, remaining, addTask, toggleDone, selectTask } from "./model";

export default function Tasks() {
  const draftTitle = signal("");

  const submit = () => {
    if (addTask(draftTitle())) {
      draftTitle.set("");
    }
  };

  const openTask = (id: number) => {
    selectTask(id);
    navigate.push("detail");
  };

  return (
    <Screen title="Tasks">
      <Section title="Add a task">
        <TextField label="Title" placeholder="What do you need to do?" value={draftTitle} onSubmit={submit} />
        <Button label="Add task" role="primary" onPress={submit} disabled={draftTitle().trim() === ""} />
      </Section>
      <Section title="All tasks">
        <Text tone="muted">{`${remaining()} of ${tasks().length} left`}</Text>
        <List
          items={tasks()}
          key={(t) => t.id}
          row={(t) => (
            <Row title={t.title} subtitle={t.done ? "Done" : "Not done"} onPress={() => openTask(t.id)}>
              <Button label={t.done ? "Undo" : "Done"} onPress={() => toggleDone(t.id)} />
            </Row>
          )}
          empty={<Empty title="No tasks yet" message="Add one above." />}
        />
      </Section>
    </Screen>
  );
}
