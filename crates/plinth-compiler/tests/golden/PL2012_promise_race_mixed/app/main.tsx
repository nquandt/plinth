import { app, Screen } from "plinth:ui";
async function n(): Promise<number> { return 1; }
async function s(): Promise<string> { return "a"; }
const p = Promise.race([n(), s()]);
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
