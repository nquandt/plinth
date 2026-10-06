import { signal, Screen, Section, TextField, Button, Text, List, Row, Toggle, Empty } from "plinth:ui";
import { tasks, visibleTasks, remaining, addTask, setDone, deleteTask } from "./model";

export default function Tasks() {
  const draft = signal("");

  const submit = () => {
    if (addTask(draft())) {
      draft.set("");
    }
  };

  return (
    <Screen title="Tasks">
      <Section title="New task">
        <TextField label="Task" placeholder="What needs to be done?" value={draft} onSubmit={submit} />
        <Button label="Add task" role="primary" onPress={submit} disabled={draft().trim() === ""} />
      </Section>
      <Section title="Tasks">
        <Text tone="muted">{`${remaining()} of ${tasks().length} remaining`}</Text>
        <List
          items={visibleTasks()}
          key={(t) => t.id}
          row={(t) => (
            <Row title={t.title} subtitle={t.done ? "Done" : "Open"}>
              <Toggle label="Done" value={t.done} onChange={(done) => setDone(t.id, done)} />
              <Button label="Delete" role="destructive" onPress={() => deleteTask(t.id)} />
            </Row>
          )}
          empty={<Empty title={tasks().length === 0 ? "No tasks yet" : "Nothing to show"} message="Add a task above." />}
        />
      </Section>
    </Screen>
  );
}
