// A keeper for one secret at a time: a name, a value, and the buttons Save,
// Load and Delete (`plinth:secrets`, core 1.13, docs/SECRETS.md). The host
// keeps the value in a safe store; no other app can read it. A host with no
// safe store rejects each call with "denied:unsupported", and the status text
// tells it. plinth.toml declares the `secrets` capability.
import { app, signal, Screen, Section, TextField, Button, Text } from "plinth:ui";
import { set, get, remove } from "plinth:secrets";

function Keeper() {
  const name = signal("api-token");
  const value = signal("");
  const status = signal("Enter a name and a value, then press Save.");

  const save = async (): Promise<void> => {
    try {
      await set(name(), value());
      status.set("Saved " + name() + ".");
    } catch (e) {
      status.set("Not saved: " + (e as Error).message);
    }
  };

  const load = async (): Promise<void> => {
    try {
      const found = await get(name());
      if (found === null) {
        status.set("There is no secret named " + name() + ".");
      } else {
        value.set(found);
        status.set("Loaded " + name() + ".");
      }
    } catch (e) {
      status.set("Not loaded: " + (e as Error).message);
    }
  };

  const erase = async (): Promise<void> => {
    try {
      await remove(name());
      value.set("");
      status.set("Deleted " + name() + ".");
    } catch (e) {
      status.set("Not deleted: " + (e as Error).message);
    }
  };

  return (
    <Screen title="Secret keeper">
      <Section title="Secret" footer="The value stays on this device, in the safe store of the system. No other app can read it.">
        <TextField label="Name" value={name} />
        <TextField label="Value" value={value} />
        <Button label="Save" role="primary" onPress={() => { save(); }} disabled={name().trim() === ""} />
        <Button label="Load" onPress={() => { load(); }} disabled={name().trim() === ""} />
        <Button label="Delete" onPress={() => { erase(); }} disabled={name().trim() === ""} />
        <Text tone="muted">{status()}</Text>
      </Section>
    </Screen>
  );
}

export default app({
  accent: "blue",
  screens: {
    keeper: { title: "Secret keeper", icon: "lock", component: Keeper },
  },
});
