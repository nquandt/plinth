import { app, Screen, Text } from "plinth:ui";

type Expr = { kind: "num"; value: number } | { kind: "add"; args: Expr[] };

function Home() {
  const e: Expr = { kind: "num", value: 1 };
  return (
    <Screen title="Home">
      <Text>{e.kind}</Text>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Home", component: Home } } });
