// `plinth:dialog`: host-owned modal dialogs (SPEC.md §8.5). No capability
// is needed. Each call returns at once; the answer reaches `done` later,
// through a `completion` event the host sends once the user responds.
import { app, signal, Screen, Section, Heading, Text, Button } from "plinth:ui";
import { alert, confirm, prompt } from "plinth:dialog";

function Dialogs() {
  const count = signal(0);
  const name = signal<string | null>(null);

  const showAlert = () => {
    // `done` is optional: nothing runs after the user dismisses this one.
    alert("This is an alert.");
  };

  const showConfirm = () => {
    confirm("Reset the count?", (ok) => {
      if (ok) {
        count.set(0);
      }
    });
  };

  const showPrompt = () => {
    prompt("Your name?", (value) => {
      if (value !== null) {
        name.set(value);
      }
    });
  };

  return (
    <Screen title="Dialogs">
      <Section title="Count">
        <Heading level={1}>{`${count()}`}</Heading>
        <Button label="Add one" role="primary" onPress={() => count.set(count() + 1)} />
      </Section>
      <Section title="Name">
        <Text tone="muted">{name() ?? "no name yet"}</Text>
      </Section>
      <Section title="Dialogs">
        <Button label="Alert" onPress={showAlert} />
        <Button label="Confirm" onPress={showConfirm} />
        <Button label="Prompt" onPress={showPrompt} />
      </Section>
    </Screen>
  );
}

export default app({
  accent: "blue",
  screens: {
    dialogs: { title: "Dialogs", icon: "number", component: Dialogs },
  },
});
