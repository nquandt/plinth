// UI API 1.14 fixture: hidden (also in partial styles), minWidth and
// minHeight, Screen layout="full", pressed and selected on a Pressable.
// `run-a11y.mjs` (`checkUi114`) loads it in Edge. The same app is in
// `crates/plinth-shoot/tests/ui114.rs`.
import { app, signal, Screen, Box, Pressable, Span } from "plinth:ui";

const on = signal(false);
const choice = signal(false);

function Home() {
  return (
    <Screen title="UI 1.14" layout="full">
      <Box label="Stack" gap={3}>
        <Pressable label="First" role="button" onPress={() => {}} height={8} />
        <Pressable label="Not on compact" role="button" onPress={() => {}} height={8} compact={{ hidden: true }} />
        <Pressable label="Not on wide" role="button" onPress={() => {}} height={8} wide={{ hidden: true }} />
        <Pressable label="Never there" role="button" onPress={() => {}} height={8} hidden={true} />
        <Span hidden={true}>Hidden words</Span>
        <Span compact={{ hidden: true }}>Words off on compact</Span>
        <Pressable label="Last" role="button" onPress={() => {}} height={8} />
      </Box>
      <Box label="Wide box" width="full" height={6} />
      <Box label="Floor row" direction="row" width={100}>
        <Box label="Floor A" grow={1} minWidth={70} minHeight={20} />
        <Box label="Floor B" grow={1} />
      </Box>
      <Pressable label="Chip" role="button" width={30} height={8} pressed={on()} onPress={() => on.set(!on())} />
      <Pressable label="Own look" role="button" width={30} height={8} bg="accent" pressed={on()} onPress={() => on.set(!on())} />
      <Pressable label="Choice" role="button" width={30} height={8} selected={choice()} onPress={() => choice.set(!choice())} />
      <Pressable label="Current page" role="link" width={30} height={8} selected={true} onPress={() => {}} />
    </Screen>
  );
}

export default app({ screens: { home: { title: "UI 1.14", component: Home } } });
