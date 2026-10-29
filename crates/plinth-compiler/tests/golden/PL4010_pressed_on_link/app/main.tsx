import { app, Screen, Pressable } from "plinth:ui";
function Home() { return <Screen title="Home"><Pressable label="Docs" role="link" pressed={true} onPress={() => {}} /></Screen>; }
export default app({ screens: { home: { title: "Home", component: Home } } });
