import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
function Home() { const x = 1; x = 2; return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
