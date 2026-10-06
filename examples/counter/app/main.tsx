import { app, signal, computed, Screen, Section, Heading, Text, Button } from "plinth:ui";

function Counter() {
  const count = signal(0);
  const parity = computed(() => (count() % 2 === 0 ? "even" : "odd"));

  return (
    <Screen title="Counter">
      <Section title="Value">
        <Heading level={1}>{count()}</Heading>
        <Text tone="muted">{parity()}</Text>
      </Section>
      <Section title="Actions">
        <Button label="Increment" role="primary" onPress={() => count.update((c) => c + 1)} />
        <Button label="Decrement" onPress={() => count.update((c) => c - 1)} />
        <Button label="Reset" role="destructive" onPress={() => count.set(0)} />
      </Section>
    </Screen>
  );
}

export default app({
  accent: "teal",
  screens: {
    counter: { title: "Counter", icon: "number", component: Counter },
  },
});
