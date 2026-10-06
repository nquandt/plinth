import { signal, computed, effect, Screen, Section, Picker, NumberField, Text } from "plinth:ui";
import { convert, round, unitsFor } from "./convert";

// `Picker.options` can now be any `string[]` expression, not just an array
// literal (gap #1; see docs/GAPS.md), so a single From/To Picker pair
// works for every kind instead of one Picker per kind.
export default function Converter() {
  const kind = signal("Length");
  const from = signal("meters");
  const to = signal("feet");
  const input = signal(1);
  const units = computed(() => unitsFor(kind()));

  // Reset the units to the new kind's first two options whenever `kind`
  // changes, so `from`/`to` always name units of the current kind.
  effect(() => {
    const list = units();
    from.set(list[0]);
    to.set(list.length > 1 ? list[1] : list[0]);
  });

  const resultText = () => {
    const result = convert(kind(), input(), from(), to());
    if (result === null) {
      return "Pick matching units.";
    }
    return `${round(input())} ${from()} = ${round(result)} ${to()}`;
  };

  return (
    <Screen title="Converter">
      <Section title="Kind">
        <Picker label="Kind" value={kind} options={["Length", "Weight", "Temperature"]} />
        <NumberField label="Value" value={input} step={0.1} />
      </Section>

      <Section title="Units">
        <Picker label="From" value={from} options={units()} />
        <Picker label="To" value={to} options={units()} />
      </Section>

      <Section title="Result">
        <Text style="mono">{resultText()}</Text>
      </Section>
    </Screen>
  );
}
