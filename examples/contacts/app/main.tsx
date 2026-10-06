import { app } from "plinth:ui";
import ContactList from "./list";
import ContactDetail from "./detail";

export default app({
  accent: "blue",
  screens: {
    list: { title: "Contacts", icon: "list", component: ContactList },
    // Not in `primary`: reachable only through `navigate.push("detail")`
    // (SPEC.md §6.2, UI API 1.2).
    detail: { title: "Contact", component: ContactDetail },
  },
  primary: ["list"],
});
