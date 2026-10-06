import { app, Screen, Box } from "plinth:ui";
function Home() { return <Screen title="Home"><Box compact={{ colour: "accent" }} /></Screen>; }
export default app({ screens: { home: { title: "Home", component: Home } } });
