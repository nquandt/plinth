import { app } from "plinth:ui";
import Tasks from "./tasks";
import TaskDetail from "./detail";

export default app({
  accent: "teal",
  screens: {
    tasks: { title: "Tasks", icon: "list", component: Tasks },
    // Not in `primary`: reachable only through `navigate.push("detail")`
    // (SPEC.md §6.2, UI API 1.2).
    detail: { title: "Task", component: TaskDetail },
  },
  primary: ["tasks"],
});
