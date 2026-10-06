import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
async function f(): Promise<boolean> { return false; }
async function g(): Promise<void> { do { await f(); } while (false); }
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
