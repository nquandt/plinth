import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
function Home() { const n: number | null = null; const y = n!; return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
