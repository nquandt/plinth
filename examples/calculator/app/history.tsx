import { Screen, Section, List, Row, Empty } from "plinth:ui";
import { history } from "./model";

export default function History() {
  return (
    <Screen title="History">
      <Section title="Past calculations">
        <List
          items={history()}
          key={(c) => c.id}
          row={(c) => <Row title={c.text} />}
          empty={<Empty title="No calculations yet" message="Use the calculator to compute something." />}
        />
      </Section>
    </Screen>
  );
}
