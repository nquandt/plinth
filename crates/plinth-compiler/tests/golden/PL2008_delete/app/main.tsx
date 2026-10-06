import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
function Home() { const o: { x: number } = { x: 1 }; delete o.x; return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
