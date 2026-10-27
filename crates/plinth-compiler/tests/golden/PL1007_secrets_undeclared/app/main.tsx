import { app, Screen } from "plinth:ui";
import { set } from "plinth:secrets";
set("token", "abc");
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
