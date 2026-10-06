import { signal, computed, Screen, Section, NumberField, TextField, Button, Text } from "plinth:ui";
import { now, formatDate, toISOString, parseDate } from "plinth:time";

// A timestamp converter (docs/GAPS.md gap #5): edit the epoch ms and see
// the local and UTC breakdown, or paste an ISO string and see its ms.
export default function Timestamps() {
  const start = now();
  const ms = signal(start);
  const text = signal(toISOString(start));

  const localLine = computed(() => "Local: " + formatDate(ms(), "YYYY-MM-DD HH:mm:ss dddd", false));
  const utcLine = computed(() => "UTC:   " + formatDate(ms(), "YYYY-MM-DD HH:mm:ss dddd", true));
  const isoLine = computed(() => "ISO:   " + toISOString(ms()));
  const parseError = computed(() => (parseDate(text()) === null ? "Not a valid date/time." : ""));

  return (
    <Screen title="Timestamps">
      <Section title="Epoch milliseconds">
        <NumberField label="ms since 1970-01-01 UTC" value={ms} step={1000} />
        <Button label="Now" onPress={() => ms.set(now())} />
      </Section>

      <Section title="Breakdown">
        <Text style="mono">{localLine()}</Text>
        <Text style="mono">{utcLine()}</Text>
        <Text style="mono">{isoLine()}</Text>
      </Section>

      <Section title="Parse an ISO date/time">
        <TextField label="ISO text" value={text} />
        <Button
          label="Parse into ms"
          onPress={() => {
            const parsed = parseDate(text());
            if (parsed !== null) {
              ms.set(parsed);
            }
          }}
        />
        <Text tone="danger">{parseError()}</Text>
      </Section>
    </Screen>
  );
}
