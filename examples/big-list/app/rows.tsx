import { signal, Screen, Section, TextField, Button, Text, List, Row as ListRow, Toggle, Badge, Empty } from "plinth:ui";
import { filter, visibleRows, setRowOn, toggleAll } from "./model";

export default function Rows() {
  const draft = signal("");

  const applyFilter = () => {
    filter.set(draft());
  };

  return (
    <Screen title="Big List">
      <Section title="Filter">
        <TextField label="Filter" placeholder="Filter rows" value={draft} onSubmit={applyFilter} />
        <Button label="Apply" onPress={applyFilter} />
        <Button label="Toggle all" onPress={() => toggleAll(true)} />
      </Section>
      <Section title="Rows">
        <Text tone="muted">{`${visibleRows().length} rows`}</Text>
        <List
          items={visibleRows()}
          key={(r) => r.id}
          row={(r) => {
            const badge = r.badge ? <Badge label="Every 10th" tone="success" /> : null;
            const toggle = <Toggle label="On" value={r.on} onChange={(on) => setRowOn(r.id, on)} />;
            const subtitle = r.subtitle;
            if (subtitle === null) {
              return (
                <ListRow title={r.title}>
                  {badge}
                  {toggle}
                </ListRow>
              );
            }
            return (
              <ListRow title={r.title} subtitle={subtitle}>
                {badge}
                {toggle}
              </ListRow>
            );
          }}
          empty={<Empty title="No rows match" message="Try a different filter." />}
        />
      </Section>
    </Screen>
  );
}
