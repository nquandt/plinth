// The library screen (`docs/HUB.md` §9.1): every app, filtered by group
// and by text, and a sheet to make a new group.
import { signal, navigate, Screen, Section, Tabs, TextField, List, Row, Empty, Text, Sheet, Button, Action, Badge } from "plinth:ui";
import { createGroup } from "plinth:hub";
import {
  reload,
  loadError,
  groupTabs,
  groupFilter,
  filterText,
  visibleApps,
  apps,
  selectedId,
  appSubtitle,
  appRisk,
  riskLabel,
  updateCount,
  updateStatus,
  checkingUpdates,
  checkForUpdates,
} from "./model";

function openApp(id: string): void {
  selectedId.set(id);
  navigate.push("app");
}

function countText(): string {
  const shown = visibleApps().length;
  const total = apps().length;
  if (shown === total) {
    return total === 1 ? "1 app" : `${total} apps`;
  }
  return `${shown} of ${total} apps`;
}

export default function Library() {
  // Components run one time (SPEC.md §5.1): this is the first load. The
  // Refresh action and every change reload again. The Hub checks for
  // updates when it starts (`docs/HUB.md` §9.2).
  reload();
  checkForUpdates();

  const newGroupOpen = signal(false);
  const newGroupName = signal("");

  const makeGroup = () => {
    const name = newGroupName().trim();
    if (name === "") {
      return;
    }
    createGroup(name);
    newGroupName.set("");
    newGroupOpen.set(false);
    reload();
    groupFilter.set(name);
  };

  return (
    <Screen
      title="Library"
      actions={[
        <Action label="Refresh" icon="refresh" onPress={reload} />,
        <Action label="Check for updates" icon="download" onPress={checkForUpdates} />,
        <Action label="New group" icon="plus" onPress={() => newGroupOpen.set(true)} />,
      ]}
    >
      {loadError() !== "" ? <Text tone="danger">{loadError()}</Text> : null}
      {updateCount() > 0 ? <Badge label={updateCount() === 1 ? "1 update available" : `${updateCount()} updates available`} tone="success" /> : null}
      {updateStatus() !== "" ? <Text tone="muted">{checkingUpdates() ? "Checking for updates…" : updateStatus()}</Text> : null}
      <Tabs items={groupTabs()} value={groupFilter} />
      <TextField label="Find in library" placeholder="App name" value={filterText} />
      <Section title="Apps" footer={countText()}>
        <List
          items={visibleApps()}
          key={(a) => a.id}
          row={(a) => (
            <Row
              title={a.name}
              subtitle={appSubtitle(a)}
              icon={a.blocked || a.publisherBlocked ? "lock" : a.update !== "" ? "download" : appRisk(a) === "high" ? "warning" : "star"}
              trailing={riskLabel(appRisk(a))}
              onPress={() => openApp(a.id)}
            />
          )}
          empty={<Empty title="No apps" message="Find apps on the Discover screen, or add one with `plinth hub add`." />}
        />
      </Section>
      <Sheet open={newGroupOpen} title="New group" onClose={() => newGroupOpen.set(false)}>
        <TextField label="Group name" placeholder="For example Work" value={newGroupName} onSubmit={makeGroup} />
        <Button label="Create group" role="primary" onPress={makeGroup} disabled={newGroupName().trim() === ""} />
      </Sheet>
    </Screen>
  );
}
