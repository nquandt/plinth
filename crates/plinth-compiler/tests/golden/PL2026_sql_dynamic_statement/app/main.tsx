import { app, Screen } from "plinth:ui";
import { sql } from "plinth:store";
const table = "notes";
sql.exec("DELETE FROM " + table);
function Home() { return <Screen title="Home" />; }
export default app({ screens: { home: { title: "Home", component: Home } } });
