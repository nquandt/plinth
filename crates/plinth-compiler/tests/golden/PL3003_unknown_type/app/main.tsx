import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
let x: Bogus;
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
