import { signal, Screen, Section, Group, Tabs, TextArea, NumberField, Text, Button } from "plinth:ui";
import { writeText, readText } from "plinth:clipboard";
import { now } from "plinth:time";
import { Math } from "plinth:core";

function wordCount(text: string): number {
  const trimmed = text.trim();
  if (trimmed === "") {
    return 0;
  }
  let count = 1;
  for (let i = 0; i < trimmed.length; i += 1) {
    if (trimmed.slice(i, i + 1) === " ") {
      count += 1;
    }
  }
  return count;
}

// `String` has no `split` or `toTitleCase` (SPEC.md std/lib.d.ts); built by
// hand with a character scan and `slice`. See docs/GAPS.md.
function toTitleCase(text: string): string {
  let result = "";
  let atWordStart = true;
  for (let i = 0; i < text.length; i += 1) {
    const ch = text.slice(i, i + 1);
    if (ch === " ") {
      result += ch;
      atWordStart = true;
    } else if (atWordStart) {
      result += ch.toUpperCase();
      atWordStart = false;
    } else {
      result += ch.toLowerCase();
    }
  }
  return result;
}

export default function TextTools() {
  const tab = signal("text");
  const input = signal("Hello Plinth World");
  const output = signal("");
  const epochInput = signal(now());
  const copiedMessage = signal("");

  const setOutput = (text: string) => {
    output.set(text);
    copiedMessage.set("");
  };

  const copyOutput = () => {
    writeText(output());
    copiedMessage.set("Copied to clipboard.");
  };

  const pasteInput = () => {
    const text = readText();
    if (text !== null) {
      input.set(text);
    }
  };

  // No Date type and no calendar formatting in `plinth:time` (SPEC.md
  // std/time.d.ts): this breaks an epoch down into elapsed days, hours,
  // minutes and seconds by hand instead of a calendar date. See
  // docs/GAPS.md.
  const elapsedBreakdown = () => {
    let totalSeconds = Math.floor(epochInput() / 1000);
    const days = Math.floor(totalSeconds / 86400);
    totalSeconds -= days * 86400;
    const hours = Math.floor(totalSeconds / 3600);
    totalSeconds -= hours * 3600;
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds - minutes * 60;
    return `${days}d ${hours}h ${minutes}m ${seconds}s since the epoch`;
  };

  return (
    <Screen title="Text tools">
      <Tabs items={["text", "time"]} value={tab} />

      {tab() === "text" ? (
        <Group axis="column">
          <Section title="Input">
            <TextArea label="Text" value={input} placeholder="Type something" />
            <Button label="Paste" onPress={pasteInput} />
            <Text tone="muted">{`${wordCount(input())} words, ${input().length} characters`}</Text>
          </Section>
          <Section title="Transform">
            <Button label="UPPERCASE" onPress={() => setOutput(input().toUpperCase())} />
            <Button label="lowercase" onPress={() => setOutput(input().toLowerCase())} />
            <Button label="Title Case" onPress={() => setOutput(toTitleCase(input()))} />
            <Button label="Trim" onPress={() => setOutput(input().trim())} />
          </Section>
          <Section title="Result" footer={copiedMessage()}>
            <Text style="mono">{output() === "" ? "(no result yet)" : output()}</Text>
            <Button label="Copy result" onPress={copyOutput} disabled={output() === ""} />
          </Section>
        </Group>
      ) : null}

      {tab() === "time" ? (
        <Section title="Timestamp converter">
          <Text tone="muted">{`Current time (ms since epoch): ${now()}`}</Text>
          <NumberField label="Epoch (ms)" value={epochInput} step={1000} />
          <Button label="Use current time" onPress={() => epochInput.set(now())} />
          <Text style="mono">{elapsedBreakdown()}</Text>
        </Section>
      ) : null}
    </Screen>
  );
}
