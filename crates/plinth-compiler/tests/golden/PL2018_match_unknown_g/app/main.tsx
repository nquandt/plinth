import { app, Screen } from "plinth:ui";
function words(s: string, re: RegExp): number { const m = s.match(re); return 0; }
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
