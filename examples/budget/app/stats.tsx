import { Screen, Section, Text, Progress, Button, Chart, navigate } from "plinth:ui";
import { Math } from "plinth:core";
import { totalsByCategory, maxCategoryMagnitude, monthlyTotalLines, income, spending, balance } from "./model";
import { categories } from "./types";
import type { Category } from "./types";

function progressFor(category: Category): number {
  const totals = totalsByCategory();
  const value = Math.abs(totals.get(category) ?? 0);
  return value / maxCategoryMagnitude();
}

function categoryLabel(category: Category): string {
  const totals = totalsByCategory();
  const value = totals.get(category) ?? 0;
  return `${category}: ${value.toFixed(2)}`;
}

// `<Chart>.data` is an array literal (SPEC.md §6.3, UI API 1.5: v1 encodes
// it at compile time), so the six fixed categories are spelled out rather
// than built with `categories.map(...)`. Each `value` still reads the
// `totalsByCategory` computed, so the chart updates when transactions
// change, like any other reactive prop.
function spendingMagnitude(category: Category): number {
  const totals = totalsByCategory();
  return Math.abs(totals.get(category) ?? 0);
}

export default function Stats() {
  return (
    <Screen title="Statistics" actions={[]}>
      <Section title="Overview">
        <Text>{`Income: ${income().toFixed(2)}`}</Text>
        <Text>{`Spending: ${spending().toFixed(2)}`}</Text>
        <Text tone={balance() >= 0 ? "success" : "danger"}>{`Balance: ${balance().toFixed(2)}`}</Text>
      </Section>
      <Section title="By category">
        {categories.map((c) => <Progress label={categoryLabel(c)} value={progressFor(c)} />)}
      </Section>
      <Section title="By month">
        {monthlyTotalLines().map((line) => <Text>{line}</Text>)}
      </Section>
      <Section title="Charts">
        <Chart
          label="Spending by category"
          kind="bar"
          data={[
            { label: "Groceries", value: spendingMagnitude("Groceries") },
            { label: "Rent", value: spendingMagnitude("Rent") },
            { label: "Transport", value: spendingMagnitude("Transport") },
            { label: "Fun", value: spendingMagnitude("Fun") },
            { label: "Salary", value: spendingMagnitude("Salary") },
            { label: "Other", value: spendingMagnitude("Other") },
          ]}
        />
        <Chart
          label="Spending share by category"
          kind="pie"
          data={[
            { label: "Groceries", value: spendingMagnitude("Groceries") },
            { label: "Rent", value: spendingMagnitude("Rent") },
            { label: "Transport", value: spendingMagnitude("Transport") },
            { label: "Fun", value: spendingMagnitude("Fun") },
            { label: "Salary", value: spendingMagnitude("Salary") },
            { label: "Other", value: spendingMagnitude("Other") },
          ]}
        />
      </Section>
      <Section title="Navigation">
        <Button label="Back to transactions" onPress={() => navigate.back()} />
      </Section>
    </Screen>
  );
}
