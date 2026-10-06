import { app, signal, Screen, Section, Heading, Text, Button } from "plinth:ui";

function Home() {
  const count = signal(0);

  return (
    <Screen title="Hello">
      <Section>
        <Heading level={1}>Hello, Plinth!</Heading>
        <Text tone="muted">Edit app/main.tsx and save. The app reloads.</Text>
      </Section>
      <Section title="Counter">
        <Text>{count() === 1 ? "You pressed the button 1 time." : `You pressed the button ${count()} times.`}</Text>
        <Button label="Press me" role="primary" onPress={() => count.update((n) => n + 1)} />
      </Section>
    </Screen>
  );
}

export default app({
  accent: "teal",
  screens: {
    home: { title: "Home", icon: "house", component: Home },
  },
});
