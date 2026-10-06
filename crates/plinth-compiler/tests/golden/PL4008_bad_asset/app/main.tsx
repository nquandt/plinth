import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
function Home() { return <Image src="missing.png" alt="x" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
