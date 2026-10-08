// A reminder: "Remind me" starts a timer; when it ends, `plinth:notify`
// shows a system notification (core 1.12, docs/host-apis.md
// "plinth:notify"). The `tag` lets "Cancel" remove the notification again.
// plinth.toml declares the `notify` capability.
import { app, signal, Screen, Section, Text, Button } from "plinth:ui";
import { setTimeout, clearTimeout } from "plinth:time";
import { show, cancel } from "plinth:notify";

const DELAY_MS = 5000;
const TAG = "reminder";

function Reminder() {
  const status = signal("No reminder.");
  const timerId = signal<number | null>(null);

  const remind = async (): Promise<void> => {
    timerId.set(null);
    try {
      const result = await show({ title: "Reminder", body: "Your 5 seconds are up.", tag: TAG });
      status.set(result === "shown" ? "The notification is shown." : "This device cannot show notifications.");
    } catch (e) {
      status.set("No notification: " + (e as Error).message);
    }
  };

  const start = () => {
    const old = timerId();
    if (old !== null) {
      clearTimeout(old);
    }
    status.set("A reminder comes in 5 seconds.");
    timerId.set(setTimeout(() => { remind(); }, DELAY_MS));
  };

  const stop = () => {
    const id = timerId();
    if (id !== null) {
      clearTimeout(id);
      timerId.set(null);
    }
    cancel(TAG);
    status.set("No reminder.");
  };

  return (
    <Screen title="Reminder">
      <Section title="Reminder">
        <Text>{status()}</Text>
        <Button label="Remind me" role="primary" onPress={start} />
        <Button label="Cancel" onPress={stop} />
      </Section>
    </Screen>
  );
}

export default app({
  accent: "blue",
  screens: {
    reminder: { title: "Reminder", icon: "bell", component: Reminder },
  },
});
