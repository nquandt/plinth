// A stopwatch: `plinth:time`'s setInterval/clearInterval drive a signal
// that the UI reads reactively (SPEC.md §8.5). `time` needs no
// capability, so plinth.toml declares none.
import { app, signal, computed, Screen, Section, Heading, Text, Button } from "plinth:ui";
import { setInterval, clearInterval } from "plinth:time";

const TICK_MS = 100;

function Stopwatch() {
  const running = signal(false);
  const elapsedMs = signal(0);
  const timerId = signal<number | null>(null);

  const seconds = computed(() => (elapsedMs() / 1000).toFixed(1));

  const start = () => {
    if (running()) {
      return;
    }
    running.set(true);
    timerId.set(
      setInterval(() => {
        elapsedMs.set(elapsedMs() + TICK_MS);
      }, TICK_MS),
    );
  };

  const stop = () => {
    if (!running()) {
      return;
    }
    running.set(false);
    const id = timerId();
    if (id !== null) {
      clearInterval(id);
      timerId.set(null);
    }
  };

  const reset = () => {
    stop();
    elapsedMs.set(0);
  };

  return (
    <Screen title="Stopwatch">
      <Section title="Elapsed">
        <Heading level={1}>{`${seconds()}s`}</Heading>
        <Text tone="muted">{running() ? "running" : "stopped"}</Text>
      </Section>
      <Section title="Actions">
        <Button label="Start" role="primary" onPress={start} />
        <Button label="Stop" onPress={stop} />
        <Button label="Reset" role="destructive" onPress={reset} />
      </Section>
    </Screen>
  );
}

export default app({
  accent: "orange",
  screens: {
    stopwatch: { title: "Stopwatch", icon: "number", component: Stopwatch },
  },
});
