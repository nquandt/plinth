// One app (`docs/HUB.md` §5.2, §7.2, §7.4, §9.2): who made it, the
// capability label with each grant, updates and versions, its groups, and
// Open, Block (the app or its publisher), Remove and Uninstall
// (`docs/STORAGE.md` §7).
import { signal, navigate, Screen, Section, Text, Badge, Button, List, Row, Toggle, Checkbox, Empty, Action, Progress, Dialog } from "plinth:ui";
import { launch, setGrant, block, unblock, setGroup, remove, pin, blockPublisher, unblockPublisher, update, uninstall } from "plinth:hub";
import {
  reload,
  notice,
  refreshApp,
  groups,
  selectedApp,
  selectedId,
  riskLabel,
  capabilityTitle,
  capabilitySubtitle,
  cannotText,
  publisherText,
  updateCapabilitiesText,
  HubCapability,
  HubVersion,
} from "./model";

function title(): string {
  const a = selectedApp();
  return a === null ? "App" : a.name;
}

function isBlocked(): boolean {
  const a = selectedApp();
  return a !== null && a.blocked;
}

function isPublisherBlocked(): boolean {
  const a = selectedApp();
  return a !== null && a.publisherBlocked;
}

function signer(): string {
  const a = selectedApp();
  return a === null ? "" : a.signer;
}

function aboutText(): string {
  const a = selectedApp();
  return a === null ? "This app is no longer in your apps." : publisherText(a);
}

function versionText(): string {
  const a = selectedApp();
  if (a === null) {
    return "";
  }
  const source = a.source === "" ? "Added from a file" : "From the store " + a.source;
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

function availableUpdate(): string {
  const a = selectedApp();
  return a === null ? "" : a.update;
}

function updateText(): string {
  const a = selectedApp();
  if (a === null || a.update === "") {
    return "";
  }
  let text = `Version ${a.update} is available from ${a.source}. ${updateCapabilitiesText(a)}`;
  if (a.pinned !== "") {
    text = text + ` The app stays on ${a.pinned} until you unpin it.`;
  }
  return text;
}

function pinned(): string {
  const a = selectedApp();
  return a === null ? "" : a.pinned;
}

function versions(): HubVersion[] {
  const a = selectedApp();
  return a === null ? [] : a.versions;
}

function versionsFooter(): string {
  const p = pinned();
  if (p === "") {
    return "The newest version runs. Select a version to keep the app on it (pin).";
  }
  return `Pinned to ${p}. Select another version to pin it, or Run the newest version.`;
}

function versionSubtitle(v: HubVersion): string {
  const a = selectedApp();
  const running = a !== null && a.version === v.version ? "Runs now" : "On this device";
  const caps = v.capabilities.length === 0 ? "no permissions" : v.capabilities.join(", ");
  return `${running} · ${caps}`;
}

/** What an uninstall deletes, for the confirmation (`docs/STORAGE.md` §7). */
function uninstallMessage(name: string, keepData: boolean): string {
  if (keepData) {
    return `${name} is deleted from this device. Your data and your secrets stay. If you install ${name} again, it finds them. You cannot undo this.`;
  }
  return `${name} is deleted from this device. Its data and its secrets (saved passwords and tokens) are deleted too. You cannot undo this. Select Keep my data to keep them.`;
}

export default function AppDetail() {
  const busy = signal(false);
  const status = signal("");
  const failed = signal(false);
  const uninstallOpen = signal(false);
  const keepData = signal(false);

  const open = () => {
    launch(selectedId());
  };

  const changeGrant = (capability: string, allowed: boolean) => {
    setGrant(selectedId(), capability, allowed);
    refreshApp(selectedId());
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
    refreshApp(selectedId());
  };

  // A publisher block changes every app of that key, so reload them all.
  const togglePublisherBlock = () => {
    if (isPublisherBlocked()) {
      unblockPublisher(signer());
    } else {
      blockPublisher(signer());
    }
    reload();
  };

  const removeApp = () => {
    remove(selectedId());
    reload();
    navigate.back();
  };

  const askUninstall = () => {
    keepData.set(false);
    uninstallOpen.set(true);
  };

  // The host runs the uninstall off the UI thread. It refuses while the app
  // runs, and it cannot uninstall the Hub itself; the error text shows here.
  const doUninstall = () => {
    const id = selectedId();
    const name = title();
    const keep = keepData();
    uninstallOpen.set(false);
    busy.set(true);
    failed.set(false);
    status.set(`Uninstalling ${name}…`);
    uninstall(id, keep, (error) => {
      busy.set(false);
      if (error === null) {
        notice.set(keep ? `${name} is uninstalled. Its data stays on this device.` : `${name} and its data are deleted.`);
        reload();
        navigate.back();
      } else {
        failed.set(true);
        status.set(`Could not uninstall: ${error}`);
        refreshApp(id);
      }
    });
  };

  const installUpdate = () => {
    const id = selectedId();
    const version = availableUpdate();
    busy.set(true);
    failed.set(false);
    status.set(`Getting version ${version}…`);
    update(id, (error) => {
      busy.set(false);
      refreshApp(id);
      if (error === null) {
        status.set(`Version ${version} is ready.`);
      } else {
        status.set(`Could not update: ${error}`);
      }
    });
  };

  const pinTo = (version: string) => {
    pin(selectedId(), version);
    refreshApp(selectedId());
    status.set(version === "" ? "The newest version runs." : `Pinned to ${version}.`);
  };

  return (
    <Screen
      title={title()}
      actions={[
        <Action label={isBlocked() ? "Unblock" : "Block"} icon="lock" onPress={toggleBlock} />,
        <Action label="Remove from your apps" icon="trash" role="destructive" onPress={removeApp} />,
        <Action label="Uninstall" icon="trash" role="destructive" confirm={false} onPress={askUninstall} />,
      ]}
    >
      <Section title="About">
        <Text>{aboutText()}</Text>
        <Text tone="muted">{versionText()}</Text>
        {isBlocked() ? <Badge label="Blocked: it will not open" tone="danger" /> : null}
        {isPublisherBlocked() ? <Badge label="Publisher blocked: no app of this publisher opens" tone="danger" /> : null}
        <Button label="Open" role="primary" onPress={open} disabled={isBlocked() || isPublisherBlocked()} />
        {signer() !== "" ? (
          <Button label={isPublisherBlocked() ? "Unblock publisher" : "Block publisher"} onPress={togglePublisherBlock} />
        ) : null}
      </Section>
      {availableUpdate() !== "" ? (
        <Section title="Update">
          <Badge label={"Update available: " + availableUpdate()} tone="success" />
          <Text>{updateText()}</Text>
          <Button label={"Update to " + availableUpdate()} role="primary" onPress={installUpdate} disabled={busy()} />
        </Section>
      ) : null}
      {busy() ? <Progress label="Working" /> : null}
      {status() !== "" ? <Text tone={failed() ? "danger" : "muted"}>{status()}</Text> : null}
      <Section title="Permissions" footer={footer()}>
        <List
          items={capabilities()}
          key={(c) => c.name}
          row={(c) => (
            <Row title={capabilityTitle(c)} subtitle={capabilitySubtitle(c)} trailing={riskLabel(c.risk)}>
              <Toggle label={"Allow: " + capabilityTitle(c)} value={c.allowed} onChange={(v) => changeGrant(c.name, v)} />
            </Row>
          )}
          empty={<Empty title="Nothing to allow" message="This app only shows its own screens." />}
        />
      </Section>
      <Section title="Versions" footer={versionsFooter()}>
        <List
          items={versions()}
          key={(v) => v.version}
          row={(v) => (
            <Row
              title={"Version " + v.version}
              subtitle={versionSubtitle(v)}
              icon={pinned() === v.version ? "lock" : "clock"}
              trailing={pinned() === v.version ? "Pinned" : ""}
              onPress={() => pinTo(v.version)}
            />
          )}
        />
        {pinned() !== "" ? <Button label="Run the newest version" onPress={() => pinTo("")} /> : null}
      </Section>
      <Section title="Groups" footer="Make a group with New group on the Your apps screen.">
        {groups().map((g) => (
          <Checkbox label={g} value={inGroup(g)} onChange={(v) => changeGroup(g, v)} />
        ))}
      </Section>
      <Dialog
        open={uninstallOpen}
        title={`Uninstall ${title()}?`}
        message={uninstallMessage(title(), keepData())}
        onClose={() => uninstallOpen.set(false)}
        actions={[
          <Action label="Cancel" onPress={() => uninstallOpen.set(false)} />,
          <Action label="Uninstall" role="destructive" confirm={false} onPress={doUninstall} />,
        ]}
      >
        <Checkbox label="Keep my data" value={keepData} />
      </Dialog>
    </Screen>
  );
}
