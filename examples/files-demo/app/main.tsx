// A small editor for text files in the app's private space (`plinth:files`,
// core 1.11, docs/host-apis.md). Each call returns a `Promise`; a failed
// call rejects with an `Error` whose message says why ("not-found",
// "invalid-path: dot segment", "denied:refused", ...).
import { app, signal, Screen, Section, TextField, TextArea, Button, Text, List, Row, Empty } from "plinth:ui";
import { read, write, list, remove } from "plinth:files";

const folder = "notes";
const names = signal<string[]>([]);
const name = signal("hello.md");
const text = signal("");
const status = signal("");

/** Reads the names of the files in the folder. */
async function refresh(): Promise<void> {
  try {
    const entries = await list(folder);
    names.set(entries.filter((e) => e.kind === "file").map((e) => e.name));
  } catch (e) {
    // The folder exists only after the first save.
    names.set([]);
    const why = (e as Error).message;
    if (why !== "not-found") status.set("Cannot list the files: " + why);
  }
}

async function save(): Promise<void> {
  try {
    await write(folder + "/" + name(), text());
    status.set("Saved " + name() + ".");
    await refresh();
  } catch (e) {
    status.set("Not saved: " + (e as Error).message);
  }
}

async function open(file: string): Promise<void> {
  try {
    text.set(await read(folder + "/" + file));
    name.set(file);
    status.set("Opened " + file + ".");
  } catch (e) {
    status.set("Cannot open " + file + ": " + (e as Error).message);
  }
}

async function erase(file: string): Promise<void> {
  try {
    await remove(folder + "/" + file);
    status.set("Deleted " + file + ".");
    await refresh();
  } catch (e) {
    status.set("Not deleted: " + (e as Error).message);
  }
}

function Files() {
  refresh();
  return (
    <Screen title="Files">
      <Section title="Edit" footer="The files stay on this device. No other app can read them.">
        <TextField label="File name" value={name} />
        <TextArea label="Text" value={text} />
        <Button label="Save" role="primary" onPress={() => { save(); }} disabled={name().trim() === ""} />
        <Text tone="muted">{status()}</Text>
      </Section>
      <Section title="Your files">
        <List
          items={names()}
          key={(n) => n}
          row={(n) => (
            <Row title={n} onPress={() => { open(n); }}>
              <Button label="Delete" role="destructive" onPress={() => { erase(n); }} />
            </Row>
          )}
          empty={<Empty title="No files yet" message="Write some text and press Save." />}
        />
      </Section>
    </Screen>
  );
}

export default app({ accent: "teal", screens: { files: { title: "Files", icon: "list", component: Files } } });
