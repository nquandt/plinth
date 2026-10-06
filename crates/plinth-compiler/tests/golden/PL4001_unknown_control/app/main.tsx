import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
function Home() { return <Bogus />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
