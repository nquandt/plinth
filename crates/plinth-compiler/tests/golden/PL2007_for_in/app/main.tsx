import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
function Home() { for (const k in {}) {} return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
