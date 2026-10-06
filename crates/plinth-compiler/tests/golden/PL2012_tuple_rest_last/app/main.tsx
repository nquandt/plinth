import { app, Screen } from "plinth:ui";
let a: [...number[], string] | null = null;
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
