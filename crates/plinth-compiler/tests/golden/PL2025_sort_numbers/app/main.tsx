import { app, Screen } from "plinth:ui";
function Home() { const xs: number[] = [10, 9, 1]; xs.sort(); return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
