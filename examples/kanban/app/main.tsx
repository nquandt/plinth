import { app } from "plinth:ui";
import Board from "./board";

export default app({
  accent: "indigo",
  screens: {
    board: { title: "Board", icon: "list", component: Board },
  },
});
