import { app, signal, Screen, Box } from "plinth:ui";
const n = signal(2);
function Home() { return <Screen title="Home"><Box hover={{ padding: n() }} /></Screen>; }
export default app({ screens: { home: { title: "Home", component: Home } } });
