import { app } from "plinth:ui";
import Converter from "./converter";
import TextTools from "./texttools";

export default app({
  accent: "purple",
  screens: {
    convert: { title: "Converter", icon: "number", component: Converter },
    text: { title: "Text tools", icon: "edit", component: TextTools },
  },
  primary: ["convert", "text"],
});
