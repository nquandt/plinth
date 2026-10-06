import { navigate, Action, Screen, Section, Text } from "plinth:ui";
import { selectedTask, deleteTask } from "./model";

// A plain helper function, not a JSX component: called directly from a
// prop, so the compiler wraps the call in an effect that reruns when
// `selected` or `tasks` changes (SPEC.md §7.1). A signal read outside such
// a position is not reactive, so handlers re-read `selectedTask()` instead
// of closing over a value read at component-call time.
function detailTitle(): string {
  const t = selectedTask();
  return t === null ? "Task" : t.title;
}

export default function TaskDetail() {
  const removeTask = () => {
    const t = selectedTask();
    if (t === null) {
      return;
    }
    deleteTask(t.id);
    navigate.back();
  };

  return (
    <Screen title={detailTitle()} actions={[<Action label="Delete" role="destructive" onPress={removeTask} />]}>
      <Section title="Status">
        <Text tone="muted">{selectedTask()?.done ? "Done" : "Not done"}</Text>
      </Section>
    </Screen>
  );
}
