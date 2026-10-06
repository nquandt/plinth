import { Screen, Section, Text, Progress, Button, navigate, List } from "plinth:ui";
import { Math } from "plinth:core";
import { totalsByCategory, maxCategoryMagnitude, income, spending, balance } from "./model";
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

export default function Stats() {
  return (
    <Screen title="Statistics" actions={[]}>
      <Section title="Overview">
        <Text>{`Income: ${income().toFixed(2)}`}</Text>
        <Text>{`Spending: ${spending().toFixed(2)}`}</Text>
        <Text tone={balance() >= 0 ? "success" : "danger"}>{`Balance: ${balance().toFixed(2)}`}</Text>
      </Section>
      <Section title="By category">
        <List
          items={categories}
          key={(c) => c}
          row={(c) => <Progress label={categoryLabel(c)} value={progressFor(c)} />}
        />
      </Section>
      <Section title="Navigation">
        <Button label="Back to transactions" onPress={() => navigate.back()} />
      </Section>
    </Screen>
  );
}
