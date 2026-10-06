// One app (`docs/HUB.md` §5.2, §7.2, §7.4): who made it, the capability
// label with each grant, its groups, and Open, Block and Remove.
import { navigate, Screen, Section, Text, Badge, Button, List, Row, Toggle, Checkbox, Empty, Action } from "plinth:ui";
import { launch, setGrant, block, unblock, setGroup, remove } from "plinth:hub";
import { reload, groups, selectedApp, selectedId, riskLabel, capabilityTitle, capabilitySubtitle, cannotText, publisherText, HubCapability } from "./model";

function title(): string {
  const a = selectedApp();
  return a === null ? "App" : a.name;
}

function isBlocked(): boolean {
  const a = selectedApp();
  return a !== null && a.blocked;
}

function aboutText(): string {
  const a = selectedApp();
  return a === null ? "This app is no longer in the library." : publisherText(a);
}

function versionText(): string {
  const a = selectedApp();
  if (a === null) {
    return "";
  }
  const source = a.source === "" ? "Added from a file" : "From the source " + a.source;
  return `Version ${a.version} · ${source} · ${a.id}`;
}

function capabilities(): HubCapability[] {
  const a = selectedApp();
  return a === null ? [] : a.capabilities;
}

function footer(): string {
  const a = selectedApp();
  return a === null ? "" : cannotText(a);
}

function inGroup(group: string): boolean {
  const a = selectedApp();
  return a !== null && a.groups.includes(group);
}

export default function AppDetail() {
  const open = () => {
    launch(selectedId());
  };

  const changeGrant = (capability: string, allowed: boolean) => {
    setGrant(selectedId(), capability, allowed);
    reload();
  };

  const changeGroup = (group: string, member: boolean) => {
    setGroup(selectedId(), group, member);
    reload();
  };

  const toggleBlock = () => {
    if (isBlocked()) {
      unblock(selectedId());
    } else {
      block(selectedId());
    }
    reload();
  };

  const removeApp = () => {
    remove(selectedId());
    reload();
    navigate.back();
  };

  return (
    <Screen
      title={title()}
      actions={[
        <Action label={isBlocked() ? "Unblock" : "Block"} icon="lock" onPress={toggleBlock} />,
        <Action label="Remove from library" icon="trash" role="destructive" onPress={removeApp} />,
      ]}
    >
      <Section title="About">
        <Text>{aboutText()}</Text>
        <Text tone="muted">{versionText()}</Text>
        {isBlocked() ? <Badge label="Blocked: it will not open" tone="danger" /> : null}
        <Button label="Open" role="primary" onPress={open} disabled={isBlocked()} />
      </Section>
      <Section title="What this app can do" footer={footer()}>
        <List
          items={capabilities()}
          key={(c) => c.name}
          row={(c) => (
            <Row title={capabilityTitle(c)} subtitle={capabilitySubtitle(c)} trailing={riskLabel(c.risk)}>
              <Toggle label={"Allow " + c.name} value={c.allowed} onChange={(v) => changeGrant(c.name, v)} />
            </Row>
          )}
          empty={<Empty title="Nothing to allow" message="This app only shows its own screens." />}
        />
      </Section>
      <Section title="Groups" footer="Make a group with New group on the Library screen.">
        {groups().map((g) => (
          <Checkbox label={g} value={inGroup(g)} onChange={(v) => changeGroup(g, v)} />
        ))}
      </Section>
    </Screen>
  );
}
