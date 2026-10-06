import { Screen, Section, Heading, Button } from "plinth:ui";
import { display, inputDigit, inputDot, chooseOperator, equals, clear } from "./model";

export default function Calculator() {
  return (
    <Screen title="Calculator">
      <Section title="Display">
        <Heading level={1}>{display()}</Heading>
      </Section>
      <Section title="Clear">
        <Button label="C" onPress={clear} />
      </Section>
      <Section title="7 8 9 /">
        <Button label="7" onPress={() => inputDigit("7")} />
        <Button label="8" onPress={() => inputDigit("8")} />
        <Button label="9" onPress={() => inputDigit("9")} />
        <Button label="÷" onPress={() => chooseOperator("/")} />
      </Section>
      <Section title="4 5 6 x">
        <Button label="4" onPress={() => inputDigit("4")} />
        <Button label="5" onPress={() => inputDigit("5")} />
        <Button label="6" onPress={() => inputDigit("6")} />
        <Button label="×" onPress={() => chooseOperator("*")} />
      </Section>
      <Section title="1 2 3 -">
        <Button label="1" onPress={() => inputDigit("1")} />
        <Button label="2" onPress={() => inputDigit("2")} />
        <Button label="3" onPress={() => inputDigit("3")} />
        <Button label="-" onPress={() => chooseOperator("-")} />
      </Section>
      <Section title="0 . = +">
        <Button label="0" onPress={() => inputDigit("0")} />
        <Button label="." onPress={inputDot} />
        <Button label="=" role="primary" onPress={equals} />
        <Button label="+" onPress={() => chooseOperator("+")} />
      </Section>
    </Screen>
  );
}
