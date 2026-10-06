import { app } from "plinth:ui";
import Converter from "./converter";
import TextTools from "./texttools";
import Timestamps from "./timestamps";

export default app({
  accent: "purple",
  screens: {
    convert: { title: "Converter", icon: "number", component: Converter },
    text: { title: "Text tools", icon: "edit", component: TextTools },
    timestamps: { title: "Timestamps", icon: "calendar", component: Timestamps },
  },
  primary: ["convert", "text", "timestamps"],
});
