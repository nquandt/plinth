import { app, Screen } from "plinth:ui";
import { write } from "plinth:files";
write("notes/a.md");
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
