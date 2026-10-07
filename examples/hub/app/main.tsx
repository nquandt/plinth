// The Plinth Hub UI (`docs/HUB.md` §4.1, phase H3 step 2): a privileged
// Plinth app. It uses the `plinth:hub` module, which the host gives only
// to a package signed by a key that it trusts as a Hub key (see this
// folder's README). The host opens each app that it launches in a new
// window (`docs/HUB.md` §4.2).
import { app } from "plinth:ui";
import Library from "./library";
import Discover from "./discover";
import AppDetail from "./detail";

export default app({
  accent: "blue",
  screens: {
    library: { title: "Your apps", icon: "house", component: Library },
    discover: { title: "Store", icon: "search", component: Discover },
    // Not in `primary`: the library and the search results push it.
    app: { title: "App", component: AppDetail },
  },
  primary: ["library", "discover"],
});
