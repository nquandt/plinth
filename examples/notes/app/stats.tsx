import { Screen, Section, Text, Button, navigate } from "plinth:ui";
import { Math } from "plinth:core";
import { notes, noteCount, totalWords } from "./model";

export default function Stats() {
  return (
    <Screen title="Stats">
      <Section title="Overview">
        <Text>{`Notes: ${noteCount()}`}</Text>
        <Text>{`Total words: ${totalWords()}`}</Text>
        <Text tone="muted">
          {noteCount() === 0 ? "You have no notes yet." : `Average words per note: ${Math.round(totalWords() / noteCount())}`}
        </Text>
      </Section>
      <Section title="Longest note">
        <Text>{longestTitle()}</Text>
      </Section>
      <Section title="Navigation">
        <Button label="Back to notes" onPress={() => navigate("notes")} />
      </Section>
    </Screen>
  );
}

function longestTitle(): string {
  const all = notes();
  if (all.length === 0) {
    return "None yet";
  }
  let longest = all[0];
  for (const n of all) {
    if (n.body.length > longest.body.length) {
      longest = n;
    }
  }
  return `${longest.title} (${longest.body.length} characters)`;
}
