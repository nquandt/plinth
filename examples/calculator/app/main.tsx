import { app } from "plinth:ui";
import Calculator from "./calculator";
import History from "./history";

export default app({
  accent: "orange",
  screens: {
    calculator: { title: "Calculator", icon: "number", component: Calculator },
    history: { title: "History", icon: "list", component: History },
  },
  primary: ["calculator", "history"],
});
