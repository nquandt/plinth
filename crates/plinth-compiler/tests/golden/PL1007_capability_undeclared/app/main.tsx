import { app, Screen } from "plinth:ui";
import { writeText } from "plinth:clipboard";
writeText("hi");
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
