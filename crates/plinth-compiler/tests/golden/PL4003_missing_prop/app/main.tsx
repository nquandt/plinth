import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
function Home() { return <Image src="a.png" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
