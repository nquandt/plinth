// 7GUIs task 4, Timer (https://eugenkiss.github.io/7guis/tasks#timer).
//
// The elapsed time grows until it reaches the duration. The slider
// changes the duration at any time: a longer duration lets the timer go
// on, a shorter one stops it. "Reset" sets the elapsed time to zero.
import { app, signal, computed, Screen, Section, Progress, Text, Slider, Button } from "plinth:ui";
import { setInterval } from "plinth:time";
import { Math } from "plinth:core";

const TICK_MS = 100;

function Timer() {
  const elapsedMs = signal<number>(0);
  const durationS = signal<number>(15);

  const durationMs = computed(() => durationS() * 1000);
  const fraction = computed(() => (durationMs() <= 0 ? 1 : Math.min(1, elapsedMs() / durationMs())));

  // One interval for the life of the screen. A tick adds time only while
  // the elapsed time is less than the duration.
  setInterval(() => {
    const limit = durationMs();
    if (elapsedMs() < limit) {
      elapsedMs.set(Math.min(limit, elapsedMs() + TICK_MS));
    }
  }, TICK_MS);

  return (
    <Screen title="Timer">
      <Section>
        <Progress label="Elapsed time" value={fraction()} />
        <Text>{`${(elapsedMs() / 1000).toFixed(1)} s`}</Text>
        <Slider label="Duration" value={durationS} min={0} max={30} step={1} />
        <Text tone="muted">{`Duration: ${durationS()} s`}</Text>
        <Button label="Reset" role="primary" onPress={() => elapsedMs.set(0)} />
      </Section>
    </Screen>
  );
}

export default app({
  accent: "purple",
  screens: {
    timer: { title: "Timer", icon: "clock", component: Timer },
  },
});
