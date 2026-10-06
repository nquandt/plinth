import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
interface Box { v: number; }
const b: Box | null = null;
const y = b.v;
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
