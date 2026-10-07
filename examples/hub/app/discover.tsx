// Search across the configured sources and install (`docs/HUB.md` §5.1,
// §5.2; `docs/REGISTRY.md` §9). Both calls run off the UI thread in the
// host and answer through a callback.
import { signal, navigate, Screen, Section, TextField, Button, Progress, Text, List, Row, Empty } from "plinth:ui";
import { JSON } from "plinth:core";
import { search, install } from "plinth:hub";
import { reload, isInstalled, selectedId } from "./model";

interface SearchHit {
  id: string;
  name: string;
  version: string;
  description: string;
  source: string;
}

interface SearchResult {
  hits: SearchHit[];
  errors: string[];
}

function hitSubtitle(h: SearchHit): string {
  const about = h.description === "" ? "" : ` · ${h.description}`;
  return `${h.version} · from ${h.source}${about}`;
}

export default function Discover() {
  const query = signal("");
  const busy = signal(false);
  const hits = signal<SearchHit[]>([]);
  const errors = signal<string[]>([]);
  const status = signal("Search the stores that this Hub knows.");

  const runSearch = () => {
    busy.set(true);
    status.set("Searching…");
    search(query().trim(), (json) => {
      busy.set(false);
      if (json === null) {
        status.set("Search is not available on this host.");
        hits.set([]);
        errors.set([]);
        return;
      }
      const result = JSON.parse<SearchResult>(json);
      if (result === null) {
        status.set("A store sent a result that this Hub cannot read.");
        return;
      }
      hits.set(result.hits);
      errors.set(result.errors);
      const n = result.hits.length;
      status.set(n === 0 ? "No apps found." : n === 1 ? "1 app found." : `${n} apps found.`);
    });
  };

  const pick = (h: SearchHit) => {
    if (isInstalled(h.id)) {
      selectedId.set(h.id);
      navigate("library");
      navigate.push("app");
      return;
    }
    busy.set(true);
    status.set(`Adding ${h.name}…`);
    install(h.id, (error) => {
      busy.set(false);
      if (error === null) {
        status.set(`${h.name} is in your apps.`);
        reload();
      } else {
        status.set(`Could not add ${h.name}: ${error}`);
      }
    });
  };

  return (
    <Screen title="Store">
      <Section title="Search">
        <TextField label="Search the store" placeholder="App name or category" value={query} onSubmit={runSearch} />
        <Button label="Search" role="primary" onPress={runSearch} disabled={busy()} />
        {busy() ? <Progress label="Working" /> : null}
        <Text tone="muted">{status()}</Text>
        <List items={errors()} key={(e) => e} row={(e) => <Row title={e} icon="warning" />} />
      </Section>
      <Section title="Results">
        <List
          items={hits()}
          key={(h) => h.source + "/" + h.id}
          row={(h) => (
            <Row
              title={h.name}
              subtitle={hitSubtitle(h)}
              icon={isInstalled(h.id) ? "check" : "download"}
              trailing={isInstalled(h.id) ? "In your apps" : "Keep"}
              onPress={() => pick(h)}
            />
          )}
          empty={<Empty title="No results" message="Type a name and press Search." />}
        />
      </Section>
    </Screen>
  );
}
