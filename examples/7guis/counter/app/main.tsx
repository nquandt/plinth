// 7GUIs task 1, Counter (https://eugenkiss.github.io/7guis/tasks#counter).
// A value that starts at 0 and a "Count" button that adds 1 to it.
import { app, signal, Screen, Section, Heading, Button } from "plinth:ui";

function Counter() {
  const count = signal(0);
  return (
    <Screen title="Counter">
      <Section>
        <Heading level={1}>{count()}</Heading>
        <Button label="Count" role="primary" onPress={() => count.update((c) => c + 1)} />
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
