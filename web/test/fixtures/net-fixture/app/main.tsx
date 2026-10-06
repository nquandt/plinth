// A fixture app for web/test/run-net.mjs: exercises `plinth:net` against a
// local HTTP server (SPEC.md §8.4, §8.5, §11). The port is fixed so the
// Node test can start a server that matches; see run-net.mjs.
import { app, Screen, Text, Button, signal } from "plinth:ui";
import { fetch } from "plinth:net";

const PORT = 18797;
const result = signal("waiting");

function Home() {
  return (
    <Screen title="Home">
      <Text>{result()}</Text>
      <Button
        label="get"
        onPress={() => {
          fetch(`http://127.0.0.1:${PORT}/hello`, null, (r) => {
            result.set(r.ok + ":" + r.status + ":" + r.text);
          });
        }}
      />
      <Button
        label="post"
        onPress={() => {
          fetch(`http://127.0.0.1:${PORT}/echo`, { method: "POST", body: "hi" }, (r) => {
            result.set(r.ok + ":" + r.text);
          });
        }}
      />
      <Button
        label="await"
        onPress={async () => {
          // async/await (SPEC.md §4.5): two requests, one after the other.
          const a = await fetch(`http://127.0.0.1:${PORT}/hello`, null);
          const b = await fetch(`http://127.0.0.1:${PORT}/echo`, { method: "POST", body: a.text });
          result.set("awaited:" + b.text);
        }}
      />
      <Button
        label="denied"
        onPress={() => {
          fetch("http://198.51.100.1/x", null, (r) => {
            result.set(r.ok + ":" + (r.error ?? "null"));
          });
        }}
      />
    </Screen>
  );
}

export default app({ screens: { home: { title: "Home", component: Home } } });
