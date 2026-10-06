// A minimal sketch of the Hub UI (`docs/HUB.md` §4.1, §15 H3 step 2): a
// list of the library's apps, with a Launch action per row, using the
// privileged `plinth:hub` module. The real Hub UI (step 2 of this phase)
// will add consent flows, grants and blocking; this example only proves
// the host plumbing: `hub.manage` is High risk and only a package the
// host trusts as a Hub key may use it (see this folder's README).
import { app, signal, Screen, Section, List, Row, Empty, Text } from "plinth:ui";
import { JSON } from "plinth:core";
import { listApps, launch, lastError } from "plinth:hub";

interface HubCapability {
  name: string;
  risk: string;
  allowed: boolean;
}

interface HubApp {
  id: string;
  name: string;
  version: string;
  signer: string;
  blocked: boolean;
  capabilities: HubCapability[];
}

function Library() {
  const apps = signal<HubApp[]>([]);
  // Empty when the last `listApps()` was not denied (SPEC.md §8.5: a
  // denied call never traps, it just has nothing useful to show).
  const error = signal("");

  // Components run one time (SPEC.md §5.1): this loads the library once,
  // when the screen first builds. A real Hub UI would re-run this after
  // `launch`/`block`/`unblock`/`setGrant` change the library.
  const json = listApps();
  const parsed = json === null ? null : JSON.parse<HubApp[]>(json);
  if (parsed === null) {
    error.set(lastError() ?? "denied");
  } else {
    apps.set(parsed);
  }

  return (
    <Screen title="Hub mini">
      <Section title="Library apps">
        {error() !== "" ? <Text tone="danger">{"plinth:hub denied: " + error()}</Text> : null}
        <List
          items={apps()}
          key={(a) => a.id}
          row={(a) => (
            <Row
              title={a.name}
              subtitle={a.blocked ? `${a.version} (blocked)` : a.version}
              onPress={() => launch(a.id)}
            />
          )}
          empty={<Empty title="No apps" message="Add an app to the library with `plinth hub add`." />}
        />
      </Section>
    </Screen>
  );
}

export default app({
  accent: "blue",
  screens: { library: { title: "Hub mini", icon: "list", component: Library } },
  primary: ["library"],
});
