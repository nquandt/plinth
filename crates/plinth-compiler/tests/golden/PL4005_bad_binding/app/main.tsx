import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
function Home() { const n = signal(1); return <Checkbox label="x" value={n} />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
