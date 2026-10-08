import { app, Screen } from "plinth:ui";
import { show } from "plinth:notify";
show({ title: "Done" });
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
