// `plinth:dialog`: host-owned modal dialogs (SPEC.md §8.5). No capability
// is needed. Each call returns at once with a `Promise`; the answer comes
// later, through a `completion` event the host sends once the user
// responds. An `async` handler waits for it with `await` (SPEC.md §4.5).
import { app, signal, Screen, Section, Heading, Text, Button } from "plinth:ui";
import { alert, confirm, prompt } from "plinth:dialog";

function Dialogs() {
  const count = signal(0);
  const name = signal<string | null>(null);

  const showAlert = () => {
    // Nothing runs after the user dismisses this one, so no `await`.
    alert("This is an alert.");
  };

  const showConfirm = async () => {
    if (await confirm("Reset the count?")) {
      count.set(0);
    }
  };

  const showPrompt = async () => {
    const value = await prompt("Your name?");
    if (value !== null) {
      name.set(value);
    }
  };

  // Three dialogs, one after the other.
  const greet = async () => {
    const value = await prompt("Who do you want to greet?");
    if (value === null || value === "") {
      return;
    }
    if (await confirm(`Greet ${value}?`)) {
      await alert(`Hello, ${value}!`);
      name.set(value);
    }
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
        <Button label="Greet" onPress={greet} />
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
