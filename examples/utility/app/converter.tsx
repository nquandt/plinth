import { signal, Screen, Section, Picker, NumberField, Text } from "plinth:ui";
import { convert, round } from "./convert";

// `Picker.options` must be an array literal (PL3001; see docs/GAPS.md), so
// the unit list cannot come from a variable that changes with `kind`. The
// workaround is one Picker per kind, shown with the matching units.
export default function Converter() {
  const kind = signal("Length");
  const lengthFrom = signal("meters");
  const lengthTo = signal("feet");
  const weightFrom = signal("kilograms");
  const weightTo = signal("pounds");
  const tempFrom = signal("Celsius");
  const tempTo = signal("Fahrenheit");
  const input = signal(1);

  const resultText = () => {
    let fromUnit = lengthFrom();
    let toUnit = lengthTo();
    if (kind() === "Weight") {
      fromUnit = weightFrom();
      toUnit = weightTo();
    } else if (kind() === "Temperature") {
      fromUnit = tempFrom();
      toUnit = tempTo();
    }
    const result = convert(kind(), input(), fromUnit, toUnit);
    if (result === null) {
      return "Pick matching units.";
    }
    return `${round(input())} ${fromUnit} = ${round(result)} ${toUnit}`;
  };

  return (
    <Screen title="Converter">
      <Section title="Kind">
        <Picker label="Kind" value={kind} options={["Length", "Weight", "Temperature"]} />
        <NumberField label="Value" value={input} step={0.1} />
      </Section>

      {kind() === "Length" ? (
        <Section title="Length units">
          <Picker label="From" value={lengthFrom} options={["meters", "feet", "miles", "kilometers"]} />
          <Picker label="To" value={lengthTo} options={["meters", "feet", "miles", "kilometers"]} />
        </Section>
      ) : null}

      {kind() === "Weight" ? (
        <Section title="Weight units">
          <Picker label="From" value={weightFrom} options={["kilograms", "pounds", "ounces", "grams"]} />
          <Picker label="To" value={weightTo} options={["kilograms", "pounds", "ounces", "grams"]} />
        </Section>
      ) : null}

      {kind() === "Temperature" ? (
        <Section title="Temperature units">
          <Picker label="From" value={tempFrom} options={["Celsius", "Fahrenheit", "Kelvin"]} />
          <Picker label="To" value={tempTo} options={["Celsius", "Fahrenheit", "Kelvin"]} />
        </Section>
      ) : null}

      <Section title="Result">
        <Text style="mono">{resultText()}</Text>
      </Section>
    </Screen>
  );
}
