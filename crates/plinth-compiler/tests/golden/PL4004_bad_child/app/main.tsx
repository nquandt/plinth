import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
function Home2() { return <Screen title="Two" />; }
function Home() { return <Home2>child</Home2>; }
export default app({ screens: { home: { title: "Home", component: Home } } });
