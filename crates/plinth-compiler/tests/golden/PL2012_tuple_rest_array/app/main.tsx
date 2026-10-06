import { app, Screen } from "plinth:ui";
let b: [string, ...[number, number]] | null = null;
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
