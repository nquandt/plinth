// An app that fails on purpose (web/test/run-stop.mjs): an uncaught error
// keeps the app running; a trap stops it, and the host shows the reason.
import { app, signal, Screen, Text, Button } from "plinth:ui";

const status = signal("start");

function deep(n: number): number {
  return deep(n + 1) + 1;
}

function Home() {
  return (
    <Screen title="Stop">
      <Text>{status()}</Text>
      <Button label="ping" onPress={() => status.set("pong " + status().length)} />
      <Button
        label="throw"
        onPress={() => {
          throw new Error("on purpose");
        }}
      />
      <Button
        label="index"
        onPress={() => {
          const xs: number[] = [];
          status.set("v" + xs[3]);
        }}
      />
      <Button label="recurse" onPress={() => status.set("" + deep(0))} />
    </Screen>
  );
}

export default app({ screens: { home: { title: "Stop", component: Home } } });
