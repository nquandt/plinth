import { app, Screen, Pressable } from "plinth:ui";
function Home() { return <Screen title="Home"><Pressable label="Go" onPress={() => {}} /></Screen>; }
export default app({ screens: { home: { title: "Home", component: Home } } });
