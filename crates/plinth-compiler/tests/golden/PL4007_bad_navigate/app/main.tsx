import { app, Screen, Text, Action, Checkbox, Image, signal, computed, effect, navigate } from "plinth:ui";
function Home() { return <Action label="Go" onPress={() => navigate("bogus")} />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
