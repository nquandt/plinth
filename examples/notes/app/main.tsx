import { app } from "plinth:ui";
import Notes from "./notes";
import Stats from "./stats";

export default app({
  accent: "purple",
  screens: {
    notes: { title: "Notes", icon: "list", component: Notes },
    stats: { title: "Stats", icon: "info", component: Stats },
  },
  primary: ["notes", "stats"],
});
