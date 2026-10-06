import { app } from "plinth:ui";
import Rows from "./rows";

export default app({
  accent: "teal",
  screens: {
    rows: { title: "Rows", icon: "list", component: Rows },
  },
  primary: ["rows"],
});
