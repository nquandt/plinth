import { navigate, Screen, Section, Toggle, Button, Text } from "plinth:ui";
import { showCompleted, completed, clearCompleted } from "./model";

export default function Settings() {
  return (
    <Screen title="Settings">
      <Section title="Display">
        <Toggle label="Show completed tasks" value={showCompleted} />
      </Section>
      <Section title="Data" footer="Completed tasks are removed for good.">
        <Button
          label={completed() === 1 ? "Clear 1 completed task" : `Clear ${completed()} completed tasks`}
          role="destructive"
          onPress={clearCompleted}
          disabled={completed() === 0}
        />
        <Button label="Back to tasks" onPress={() => navigate("tasks")} />
      </Section>
      <Section title="About">
        <Text>A Plinth TS app: one source, compiled to a small Wasm component.</Text>
      </Section>
    </Screen>
  );
}
