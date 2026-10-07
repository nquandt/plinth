// 7GUIs task 2, Temperature converter
// (https://eugenkiss.github.io/7guis/tasks#temp).
//
// Two text fields, Celsius and Fahrenheit. A number typed in one field
// updates the other field. Text that is not a number stays in its field,
// shows an error, and does not change the other field.
//
// Each field has a one-way `value` and an `onChange` handler: the handler
// keeps the text that the user typed and, if it is a number, sets the
// other field. The guest does not send back a value that the host already
// shows, so the two fields do not loop.
import { app, signal, computed, Screen, Section, TextField, Text } from "plinth:ui";
import { parseNumber, toString, Math } from "plinth:core";

/** The number in `text`, or null if `text` is empty or not a number. */
function parseTemperature(text: string): number | null {
  const t = text.trim();
  if (t === "") {
    return null;
  }
  const n = parseNumber(t);
  if (n !== n) {
    return null; // NaN
  }
  return n;
}

/** A temperature with at most one decimal. */
function formatTemperature(n: number): string {
  return toString(Math.round(n * 10) / 10);
}

function errorFor(text: string): string {
  return text.trim() !== "" && parseTemperature(text) === null ? "Not a number" : "";
}

function Converter() {
  const celsius = signal("");
  const fahrenheit = signal("");
  const celsiusError = computed(() => errorFor(celsius()));
  const fahrenheitError = computed(() => errorFor(fahrenheit()));

  const changeCelsius = (text: string) => {
    celsius.set(text);
    const c = parseTemperature(text);
    if (c !== null) {
      fahrenheit.set(formatTemperature((c * 9) / 5 + 32));
    }
  };

  const changeFahrenheit = (text: string) => {
    fahrenheit.set(text);
    const f = parseTemperature(text);
    if (f !== null) {
      celsius.set(formatTemperature(((f - 32) * 5) / 9));
    }
  };

  return (
    <Screen title="Temperature Converter">
      <Section>
        <TextField label="Celsius" value={celsius()} error={celsiusError()} onChange={changeCelsius} />
        <TextField label="Fahrenheit" value={fahrenheit()} error={fahrenheitError()} onChange={changeFahrenheit} />
        <Text tone="muted">Type a number in one field to convert it.</Text>
      </Section>
    </Screen>
  );
}

export default app({
  accent: "orange",
  screens: {
    converter: { title: "Temperature", icon: "number", component: Converter },
  },
});
