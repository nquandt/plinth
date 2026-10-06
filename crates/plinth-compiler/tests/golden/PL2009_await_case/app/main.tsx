import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
async function f(): Promise<number> { return 1; }
async function g(n: number): Promise<void> { switch (n) { case await f(): break; } }
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
