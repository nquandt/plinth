import { app, Screen } from "plinth:ui";
const a: [string, ...number[]] = ["a", 1];
const i = 1; const v = a[i];
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
