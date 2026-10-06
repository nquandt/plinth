import { app } from "plinth:ui";
import Transactions from "./transactions";
import Stats from "./stats";

export default app({
  accent: "green",
  screens: {
    budget: { title: "Budget", icon: "number", component: Transactions },
    stats: { title: "Statistics", icon: "list", component: Stats },
  },
  primary: ["budget"],
});
