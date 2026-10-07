// The Hub UI state (`docs/HUB.md` §4.1, §9.1). The library lives in the
// host; this module keeps a copy in signals and reloads it after each
// change, because `plinth:hub` calls return at once and do not notify.
import { signal, computed } from "plinth:ui";
import { JSON } from "plinth:core";
import { listApps, listGroups, appInfo, checkUpdates, lastError } from "plinth:hub";

export interface HubCapability {
  name: string;
  risk: string;
  description: string;
  rationale: string;
  decided: boolean;
  allowed: boolean;
  byDefault: boolean;
}

export interface HubVersion {
  version: string;
  signer: string;
  capabilities: string[];
}

export interface HubApp {
  id: string;
  name: string;
  version: string;
  publisher: string;
  signer: string;
  source: string;
  blocked: boolean;
  publisherBlocked: boolean;
  groups: string[];
  capabilities: HubCapability[];
  /** The pinned version, or "" (`docs/HUB.md` §9.2). */
  pinned: string;
  /** The installed versions, newest first. */
  versions: HubVersion[];
  /** The newer version that the last update check found, or "". */
  update: string;
  /** The capabilities that the update adds. */
  updateCapabilities: string[];
}

interface UpdateHit {
  id: string;
  name: string;
  current: string;
  version: string;
  source: string;
  newCapabilities: string[];
  pinned: string;
}

interface UpdateCheck {
  updates: UpdateHit[];
  errors: string[];
}

/** The tab that shows every app, whatever its groups. */
export const ALL = "All";

export const apps = signal<HubApp[]>([]);
export const groups = signal<string[]>([]);
/** Empty, or the reason the last reload failed. */
export const loadError = signal("");
/** The app that the detail screen shows. */
export const selectedId = signal("");
/** `ALL` or a group name: the selected tab on the library screen. */
export const groupFilter = signal(ALL);
/** Text that the library list must contain (name or id). */
export const filterText = signal("");

/** The result of the last update check, for the library screen. */
export const updateStatus = signal("");
export const checkingUpdates = signal(false);

/** The apps that have an update (`docs/HUB.md` §9.2). */
export const updateCount = computed(() => apps().filter((a) => a.update !== "").length);

/** Asks the host to check the sources of every library app for a newer
 * version. The host keeps the result, so `reload` shows it. */
export function checkForUpdates(): void {
  checkingUpdates.set(true);
  updateStatus.set("Checking for updates…");
  checkUpdates("", (json) => {
    checkingUpdates.set(false);
    if (json === null) {
      updateStatus.set("This host cannot check for updates.");
      return;
    }
    const result = JSON.parse<UpdateCheck>(json);
    reload();
    const n = updateCount();
    let text = n === 0 ? "All apps are up to date." : n === 1 ? "1 update is available." : `${n} updates are available.`;
    if (result !== null && result.errors.length > 0) {
      text = text + " Some stores did not answer: " + result.errors.join("; ");
    }
    updateStatus.set(text);
  });
}

/** Reads one app from the host again (`appInfo`) and replaces it in the
 * list; removes it if the host no longer has it. */
export function refreshApp(id: string): void {
  const json = appInfo(id);
  const parsed = json === null ? null : JSON.parse<HubApp>(json);
  if (parsed === null) {
    apps.set(apps().filter((a) => a.id !== id));
    return;
  }
  const fresh: HubApp = parsed;
  if (apps().some((a) => a.id === id)) {
    apps.set(apps().map((a) => (a.id === id ? fresh : a)));
  } else {
    apps.set([...apps(), fresh]);
  }
}

/** Reads the library and the groups from the host again. */
export function reload(): void {
  const json = listApps();
  if (json === null) {
    loadError.set("The host refused to show your apps (" + (lastError() ?? "denied") + ").");
    apps.set([]);
  } else {
    const parsed = JSON.parse<HubApp[]>(json);
    if (parsed === null) {
      loadError.set("The host sent a list of apps that this Hub cannot read.");
      apps.set([]);
    } else {
      loadError.set("");
      apps.set(parsed);
    }
  }
  const groupsJson = listGroups();
  const parsedGroups = groupsJson === null ? null : JSON.parse<string[]>(groupsJson);
  groups.set(parsedGroups === null ? [] : parsedGroups);
  if (groupFilter() !== ALL && !groups().includes(groupFilter())) {
    groupFilter.set(ALL);
  }
}

export const groupTabs = computed(() => [ALL, ...groups()]);

export const visibleApps = computed(() => {
  const group = groupFilter();
  const needle = filterText().trim().toLowerCase();
  return apps().filter((a) => {
    if (group !== ALL && !a.groups.includes(group)) {
      return false;
    }
    if (needle === "") {
      return true;
    }
    return a.name.toLowerCase().includes(needle) || a.id.toLowerCase().includes(needle);
  });
});

export function selectedApp(): HubApp | null {
  const id = selectedId();
  const found = apps().find((a) => a.id === id);
  if (found === undefined) {
    return null;
  }
  return found;
}

export function isInstalled(id: string): boolean {
  return apps().some((a) => a.id === id);
}

// -- The capability label (`docs/HUB.md` §7.2) ---------------------------

function riskRank(risk: string): number {
  if (risk === "high") {
    return 3;
  }
  if (risk === "medium") {
    return 2;
  }
  if (risk === "low") {
    return 1;
  }
  return 0;
}

/** The words for a risk level. They are the same for every app. */
export function riskLabel(risk: string): string {
  if (risk === "high") {
    return "High risk";
  }
  if (risk === "medium") {
    return "Medium risk";
  }
  if (risk === "low") {
    return "Low risk";
  }
  return "No risk";
}

/** The highest risk of the capabilities of `app`. */
export function appRisk(app: HubApp): string {
  let best = "none";
  for (const c of app.capabilities) {
    if (riskRank(c.risk) > riskRank(best)) {
      best = c.risk;
    }
  }
  return best;
}

/** The fixed description, with a capital letter: "Save data on this device". */
export function capabilityTitle(c: HubCapability): string {
  const text = c.description === "" ? "Use " + c.name : c.description;
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** The risk, the decision and the app's own reason, for one capability. */
export function capabilitySubtitle(c: HubCapability): string {
  let decision = "Not decided yet: the Hub asks when you open the app";
  if (c.decided) {
    decision = c.allowed ? "Allowed" : "Not allowed";
    if (c.byDefault) {
      decision = decision + " by default";
    }
  }
  const reason = c.rationale === "" ? "" : ` · The app says: "${c.rationale}"`;
  return `${riskLabel(c.risk)} · ${c.name} · ${decision}${reason}`;
}

/** What the app cannot do (`docs/HUB.md` §1.1: "It cannot use the network"). */
export function cannotText(app: HubApp): string {
  const names = app.capabilities.map((c) => c.name);
  const parts: string[] = [];
  if (!names.some((n) => n.startsWith("net"))) {
    parts.push("use the network");
  }
  if (!names.includes("clipboard.read")) {
    parts.push("read the clipboard");
  }
  if (!names.includes("store.kv")) {
    parts.push("save data on this device");
  }
  parts.push("read your files");
  return "This app cannot " + parts.join(", ") + ".";
}

/** Who made the app, and whether a signature proves it (`docs/HUB.md` §6.1, §6.2). */
export function publisherText(app: HubApp): string {
  if (app.signer !== "") {
    return `Signed by ${app.publisher} (${app.signer})`;
  }
  if (app.publisher === "") {
    return "Unverified publisher";
  }
  return `${app.publisher} (unverified publisher)`;
}

/** The library row's second line. */
export function appSubtitle(app: HubApp): string {
  const source = app.source === "" ? "added from a file" : "from " + app.source;
  let state = app.blocked ? " · Blocked" : app.publisherBlocked ? " · Publisher blocked" : "";
  if (app.pinned !== "") {
    state = state + " · Pinned";
  }
  if (app.update !== "") {
    state = state + ` · Update available: ${app.update}`;
  }
  return `${app.version} · ${source}${state}`;
}

/** The words for the capabilities that an update adds. */
export function updateCapabilitiesText(app: HubApp): string {
  if (app.updateCapabilities.length === 0) {
    return "It asks for nothing new.";
  }
  return "It also asks for: " + app.updateCapabilities.join(", ") + ". The Hub asks you before the new version opens.";
}
