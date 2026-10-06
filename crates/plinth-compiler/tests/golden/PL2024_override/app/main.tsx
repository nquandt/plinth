import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
class A { m(x: number): number { return x; } }
class B extends A { m(x: string): string { return x; } }
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
