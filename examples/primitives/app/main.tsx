// Level 2 styled primitives (UI API 1.6, docs/UI-ADVANCED.md phase U1):
// Box, Span, Pressable and Scroll inside a semantic Screen and Section,
// with state styles (`hover`, `focus`) and a width-class style (`compact`,
// UI API 1.7).
// Spaces and sizes are spacing units (4 px); colors are theme tokens.
import { Math } from "plinth:core";
import { app, signal, Screen, Section, Box, Span, Pressable, Scroll, Canvas, rect, circle, line, canvasText, Shape } from "plinth:ui";

type Card = { title: string; body: string };

const CARDS: Card[] = [
  { title: "Inbox", body: "Three new messages since this morning." },
  { title: "Calendar", body: "Team meeting at ten, lunch at noon." },
  { title: "Notes", body: "Buy milk. Call the dentist. Read chapter four of the book before Friday." },
  { title: "Photos", body: "Twelve new photos from the trip." },
  { title: "Music", body: "A new album from an artist that you follow." },
  { title: "Weather", body: "Sunny, with a light wind from the west." },
];

const BARS: number[] = [3, 7, 4, 9, 6];

/** A small scene for the Canvas (UI API 1.10): view units, theme colors. */
function scene(presses: number): Shape[] {
  const shapes: Shape[] = [
    rect(0, 0, 200, 100, "surface.alt"),
    circle(170, 25, 12 + Math.min(presses, 8), "accent"),
    line(0, 80, 200, 80, "border", 2),
  ];
  const bars = BARS.map((v, i) => rect(14 + i * 22, 80 - v * 6, 14, v * 6, i % 2 === 0 ? "success" : "danger"));
  return [...shapes, ...bars, canvasText(8, 96, "Canvas: view 200 x 100", "text.muted", 9)];
}

function Gallery() {
  const presses = signal(0);
  const chosen = signal("none");

  return (
    <Screen title="Primitives">
      <Section title="Box and Span">
        <Box direction="row" gap={3} align="center" padding={3} bg="surface.alt" radius="md" compact={{ direction: "column", align: "start" }}>
          <Box width={10} height={10} radius="full" bg="accent" />
          <Box grow={1} gap={1}>
            <Span size="lg" weight="semibold">Styled primitives</Span>
            <Span fg="text.muted" lines={1}>Layout, space and color from theme tokens, on every host.</Span>
          </Box>
          <Span mono={true} fg="accent">U1</Span>
        </Box>
        <Box direction="row" gap={2} wrap={true}>
          <Box padding={2} radius="sm" border="border" width="1/3"><Span size="xs">one third</Span></Box>
          <Box padding={2} radius="sm" border="border" grow={1}><Span size="xs">grow</Span></Box>
          <Box padding={2} radius="sm" border="danger"><Span size="xs" fg="danger" italic={true}>danger</Span></Box>
        </Box>
      </Section>
      <Section title="Pressable">
        <Box direction="row" gap={2} justify="between" align="center">
          <Pressable label="Press me" role="button" paddingX={4} paddingY={2} radius="lg" bg="accent" onPress={() => presses.update((n) => n + 1)}>
            <Span fg="on.accent" weight="medium">Press me</Span>
          </Pressable>
          <Span>{`Pressed ${presses()} times`}</Span>
        </Box>
        <Pressable label="Reset the count" role="link" padding={1} onPress={() => presses.set(0)} disabled={presses() === 0}>
          <Span fg="accent">Reset the count</Span>
        </Pressable>
      </Section>
      <Section title="Canvas">
        <Canvas label="A sun over five bars" viewWidth={200} viewHeight={100} maxWidth={120} shapes={scene(presses())} />
      </Section>
      <Section title="Scroll">
        <Scroll label="Cards" direction="row" gap={3} paddingY={1}>
          {CARDS.map((card) => (
            <Pressable
              label={card.title}
              role="button"
              width={40}
              padding={3}
              gap={1}
              radius="md"
              border="border"
              bg={chosen() === card.title ? "selected" : "surface"}
              hover={{ border: "accent" }}
              focus={{ border: "accent" }}
              onPress={() => chosen.set(card.title)}
            >
              <Span weight="semibold">{card.title}</Span>
              <Span size="sm" fg="text.muted" lines={2}>{card.body}</Span>
            </Pressable>
          ))}
        </Scroll>
        <Span fg="text.muted">{`Chosen: ${chosen()}`}</Span>
      </Section>
    </Screen>
  );
}

export default app({
  accent: "indigo",
  screens: {
    gallery: { title: "Primitives", icon: "star", component: Gallery },
  },
});
