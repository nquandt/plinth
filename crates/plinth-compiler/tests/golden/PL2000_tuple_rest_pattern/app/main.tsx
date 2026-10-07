import { app, Screen, Text } from "plinth:ui";
function Home() {
  const t: [string, number] = ["a", 1];
  const [s, ...r] = t;
  return <Screen title="Home"><Text>{s}</Text></Screen>;
}
export default app({ screens: { home: { title: "Home", component: Home } } });
