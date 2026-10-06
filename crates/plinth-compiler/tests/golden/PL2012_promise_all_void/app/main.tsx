import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
async function f(): Promise<number> { return 1; }
async function g(): Promise<void> {}
const p = Promise.all([f(), g()]);
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
