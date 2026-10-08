// plinth:locale (core 1.13, docs/I18N.md §3.1): web/test/run-locale.mjs.
// The same app as crates/plinth-compiler/tests/locale.rs.
import { app, signal, computed, effect, Screen, Text } from "plinth:ui";
import { console } from "plinth:core";
import { locale, direction } from "plinth:locale";
import type { Direction } from "plinth:locale";

const runs = signal(0);

function Home() {
  const side = computed(() => (direction() === "rtl" ? "right" : "left"));
  effect(() => {
    const tag = locale();
    runs.update((n) => n + 1);
    console.log("locale " + tag);
  });
  const d: Direction = direction();
  return (
    <Screen title="Home">
      <Text>{`${locale()} ${direction()}`}</Text>
      <Text>{side()}</Text>
      <Text>{`first ${d}`}</Text>
      <Text>{`runs ${runs()}`}</Text>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Home", component: Home } } });
