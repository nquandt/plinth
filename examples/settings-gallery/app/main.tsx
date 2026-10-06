import { app, signal, Screen, Section, Text, Checkbox, TextArea } from "plinth:ui";

function Settings() {
  const notifications = signal(true);
  const bio = signal("");

  return (
    <Screen title="Settings">
      <Section title="Notifications" footer="Choose what you want to hear about.">
        <Checkbox label="Email notifications" value={notifications} />
        <Text tone="muted">{notifications() ? "You will get emails." : "Emails are off."}</Text>
      </Section>
      <Section title="Profile">
        <TextArea label="Bio" value={bio} placeholder="Tell us about yourself" />
      </Section>
    </Screen>
  );
}

export default app({
  accent: "indigo",
  screens: {
    settings: { title: "Settings", icon: "gear", component: Settings },
  },
});
