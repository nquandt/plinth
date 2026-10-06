// A small allocation-heavy app used by `gc_stress.rs` (SPEC.md §16) to
// exercise the collector: string concatenation in a loop, a `Map`/`Set`
// built from scratch on every recompute, an array of structs, closures
// captured per list row, and a region that switches its content.
import { app, signal, computed, Screen, Section, Text, Heading, Button, List, Row } from "plinth:ui";

interface Item {
  id: number;
  label: string;
}

function buildLabel(n: number): string {
  let s = "";
  for (let i = 0; i < n; i++) {
    s = s + "x";
  }
  return s;
}

function bumpFor(it: Item): number {
  return it.id * 2 + 1;
}

function Torture() {
  const seed = signal(0);
  const toggle = signal(false);

  const items = computed<Item[]>(() => {
    const seen = new Map<number, string>();
    const tags = new Set<string>();
    const out: Item[] = [];
    for (let i = 0; i < 24; i++) {
      tags.add(`tag-${i % 3}`);
      const label = buildLabel(i + 1) + `-${seed()}`;
      seen.set(i, label);
      out.push({ id: i + seed() * 100, label });
    }
    return out.filter((it) => tags.has(`tag-${it.id % 3}`) && seen.has(it.id - seed() * 100));
  });

  const grow = () => seed.update((s) => s + 1);
  const flip = () => toggle.update((t) => !t);

  return (
    <Screen title="Torture">
      <Section title="Controls">
        <Button label="Grow" role="primary" onPress={grow} />
        <Button label="Flip" onPress={flip} />
      </Section>
      <Section title="Content">
        {toggle() ? <Heading level={2}>{`seed ${seed()}`}</Heading> : <Text>{`items ${items().length}`}</Text>}
      </Section>
      <Section title="List">
        <List
          items={items()}
          key={(it) => it.id}
          row={(it) => (
            <Row title={it.label}>
              <Button label="Bump" onPress={() => seed.update((s) => s + bumpFor(it))} />
            </Row>
          )}
        />
      </Section>
    </Screen>
  );
}

export default app({
  accent: "orange",
  screens: {
    torture: { title: "Torture", icon: "star", component: Torture },
  },
});
