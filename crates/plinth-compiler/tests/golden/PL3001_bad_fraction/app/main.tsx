import { app, Screen, Box } from "plinth:ui";
function Home() { return <Screen title="Home"><Box width="1/5" /></Screen>; }
export default app({ screens: { home: { title: "Home", component: Home } } });
