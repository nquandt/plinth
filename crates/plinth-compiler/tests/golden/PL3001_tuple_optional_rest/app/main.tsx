import { app, Screen } from "plinth:ui";
const a: [number, number, string?] = [1];
const b: [number, string?] = [1, "a", "b"];
const c: [number, ...number[]] = [...[1]];
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
