import { Screen, Section, Group, Heading, Text, Button } from "plinth:ui";
import { toString } from "plinth:core";
import { display, stored, pending, inputDigit, inputDot, chooseOperator, equals, clear } from "./model";

/** One digit key. */
function Digit(props: { digit: string }) {
  return <Button label={props.digit} size="large" onPress={() => inputDigit(props.digit)} />;
}

/** The line above the display: the stored value and the pending operator. */
function pendingLine(): string {
  const op = pending();
  if (op === null) {
    return " ";
  }
  const symbol = op === "*" ? "×" : op === "/" ? "÷" : op === "-" ? "−" : "+";
  return `${toString(stored())} ${symbol}`;
}

export default function Calculator() {
  return (
    <Screen title="Calculator">
      <Section>
        <Text tone="muted" align="end">{pendingLine()}</Text>
        <Heading level={1} align="end">{display()}</Heading>
      </Section>
      <Section>
        <Group axis="row">
          <Button label="C" role="destructive" size="large" onPress={clear} />
          <Button label="." size="large" onPress={inputDot} />
          <Button label="÷" role="primary" size="large" onPress={() => chooseOperator("/")} />
        </Group>
        <Group axis="row">
          <Digit digit="7" />
          <Digit digit="8" />
          <Digit digit="9" />
          <Button label="×" role="primary" size="large" onPress={() => chooseOperator("*")} />
        </Group>
        <Group axis="row">
          <Digit digit="4" />
          <Digit digit="5" />
          <Digit digit="6" />
          <Button label="−" role="primary" size="large" onPress={() => chooseOperator("-")} />
        </Group>
        <Group axis="row">
          <Digit digit="1" />
          <Digit digit="2" />
          <Digit digit="3" />
          <Button label="+" role="primary" size="large" onPress={() => chooseOperator("+")} />
        </Group>
        <Group axis="row">
          <Digit digit="0" />
          <Button label="=" role="primary" size="large" onPress={equals} />
        </Group>
      </Section>
    </Screen>
  );
}
