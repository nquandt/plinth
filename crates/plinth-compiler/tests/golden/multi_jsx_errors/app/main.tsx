import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
function Home() { return <Text bogus="x" other="y">hi</Text>; }
export default app({ screens: { home: { title: "Home", component: Home } } });
