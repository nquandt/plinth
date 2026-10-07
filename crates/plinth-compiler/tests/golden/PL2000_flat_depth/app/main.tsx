import { app, Screen } from "plinth:ui";
const n = 2;
const b = [[[1]]].flat(n);
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
