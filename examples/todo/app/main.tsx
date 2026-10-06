import { app } from "plinth:ui";
import Tasks from "./tasks";
import Settings from "./settings";

export default app({
  accent: "teal",
  screens: {
    tasks: { title: "Tasks", icon: "list", component: Tasks },
    settings: { title: "Settings", icon: "gear", component: Settings },
  },
  primary: ["tasks", "settings"],
});
