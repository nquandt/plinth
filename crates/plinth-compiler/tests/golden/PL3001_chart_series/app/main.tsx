import { app, Screen, Chart } from "plinth:ui";
const s = [{ name: "a", points: [1, 2] }];
function Home() { return <Screen title="Home"><Chart label="c" kind="bar" data={[]} series={s} /></Screen>; }
export default app({ screens: { home: { title: "Home", component: Home } } });
