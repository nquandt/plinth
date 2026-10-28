import { app, signal, Screen, Section, Box, Span, Button } from "plinth:ui";

// A themed app: the brand colors are in plinth.toml ([theme]). This code
// uses token names only ("accent", "surface.alt", ...). The host paints
// them with the brand values, in light and in dark.
function Home() {
  const orders = signal(3);
  return (
    <Screen title="Brew & Bean">
      <Section title="Today">
        <Box direction="row" gap={3} align="center" padding={3} bg="surface.alt" radius="md" border="border">
          <Box width={10} height={10} radius="full" bg="accent" />
          <Box grow={1} gap={1}>
            <Span size="lg" weight="semibold">Open orders</Span>
            <Span fg="text.muted">Brand colors from the app theme.</Span>
          </Box>
          <Span size="lg" weight="semibold" fg="accent">{`${orders()}`}</Span>
        </Box>
        <Box direction="row" gap={2}>
          <Box grow={1} padding={2} radius="sm" bg="surface" border="border"><Span fg="success">Ready</Span></Box>
          <Box grow={1} padding={2} radius="sm" bg="surface" border="border"><Span fg="danger">Late</Span></Box>
        </Box>
      </Section>
      <Section title="Actions">
        <Button label="New order" role="primary" onPress={() => orders.update((n) => n + 1)} />
        <Button label="Close one" onPress={() => orders.update((n) => (n > 0 ? n - 1 : 0))} />
      </Section>
    </Screen>
  );
}

export default app({
  screens: {
    home: { title: "Brew & Bean", icon: "star", component: Home },
  },
});
