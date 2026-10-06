import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
function Home() { const s = signal(1); const x = s(); return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
