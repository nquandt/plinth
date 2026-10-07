// A fixture app for web/test/run-files.mjs: `plinth:files` on the private
// space (core 1.11, docs/STORAGE.md §2, §3, §6 item 5). Each button sets the
// text to "done:<result>".
import { app, Screen, Text, Button, signal } from "plinth:ui";
import { JSON } from "plinth:core";
import { read, write, list, stat, remove } from "plinth:files";

const result = signal("waiting");

// Paths that the host must refuse (docs/host-apis.md "plinth:files").
const TRICKS: string[] = [
  "../dev.plinth.test.files-fixture/secret.md",
  "../../secret.md",
  "/secret.md",
  "C:/secret.md",
  "c:secret.md",
  "a\\secret.md",
  "./secret.md",
  "a//secret.md",
  "secret.md/",
  "secret.md\u0000",
  "nul.txt",
  "",
];

async function writeSecret(): Promise<void> {
  await write("secret.md", "A's secret");
  result.set("done:written");
}

async function readSecret(): Promise<void> {
  try {
    const t = await read("secret.md");
    result.set("done:" + t);
  } catch (e) {
    result.set("done:err:" + e.message);
  }
}

async function listRoot(): Promise<void> {
  try {
    const entries = await list("");
    result.set("done:list:" + entries.length);
  } catch (e) {
    result.set("done:err:" + e.message);
  }
}

async function tricks(): Promise<void> {
  let r = "";
  for (const p of TRICKS) {
    try {
      const t = await read(p);
      r = r + "read:" + t + ";";
    } catch (e) {
      r = r + e.message + ";";
    }
  }
  try {
    await write("../escape.txt", "x");
    r = r + "escaped;";
  } catch (e) {
    r = r + e.message + ";";
  }
  result.set("done:" + r);
}

async function roundTrip(): Promise<void> {
  await write("notes/a.md", "hello");
  await write("notes/b.md", "wörld");
  const t = await read("notes/b.md");
  const root = await list("");
  const notes = await list("notes");
  const s = await stat("notes/a.md");
  const none = await stat("nope");
  await remove("notes");
  const after = await list("");
  result.set(
    "done:" + t + "|" + JSON.stringify(root) + "|" + JSON.stringify(notes) + "|" + (s === null ? "null" : s.kind + s.size) + "|" +
      (none === null ? "null" : "x") + "|" + after.length,
  );
}

function callbacks(): void {
  write("cb.md", "cb", (e) => {
    read("cb.md", (e2, text) => {
      read("missing.md", (e3, t3) => {
        result.set("done:" + (e ?? "null") + "|" + text + "|" + (e3 ?? "null") + "|" + t3.length);
      });
    });
  });
}

function Home() {
  return (
    <Screen title="Home">
      <Text>{result()}</Text>
      <Button label="write" onPress={() => { writeSecret(); }} />
      <Button label="read" onPress={() => { readSecret(); }} />
      <Button label="list" onPress={() => { listRoot(); }} />
      <Button label="tricks" onPress={() => { tricks(); }} />
      <Button label="round" onPress={() => { roundTrip(); }} />
      <Button label="callbacks" onPress={() => { callbacks(); }} />
    </Screen>
  );
}

export default app({ screens: { home: { title: "Home", component: Home } } });
