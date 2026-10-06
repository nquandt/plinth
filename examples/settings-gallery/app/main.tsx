import {
  app,
  signal,
  Screen,
  Section,
  Text,
  Checkbox,
  TextArea,
  Slider,
  NumberField,
  Picker,
  Progress,
  Badge,
} from "plinth:ui";

function Settings() {
  const notifications = signal(true);
  const bio = signal("");
  const volume = signal(40);
  const quantity = signal(3);
  const theme = signal("system");
  const plan = signal("free");

  return (
    <Screen title="Settings">
      <Section title="Notifications" footer="Choose what you want to hear about.">
        <Checkbox label="Email notifications" value={notifications} />
        <Text tone="muted">{notifications() ? "You will get emails." : "Emails are off."}</Text>
      </Section>
      <Section title="Profile">
        <TextArea label="Bio" value={bio} placeholder="Tell us about yourself" />
      </Section>
      <Section title="Sound">
        <Slider label="Volume" value={volume} min={0} max={100} step={5} />
        <NumberField label="Items" value={quantity} min={0} max={10} step={1} />
      </Section>
      <Section title="Appearance">
        <Picker label="Theme" value={theme} options={["system", "light", "dark"]} />
        <Picker label="Plan" value={plan} options={["free", "pro", "team", "enterprise", "custom"]} />
      </Section>
      <Section title="Status">
        <Progress label="Sync progress" value={volume() / 100} />
        <Progress label="Working" />
        <Badge label={plan()} tone="default" />
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
