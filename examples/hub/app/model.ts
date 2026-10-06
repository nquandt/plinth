// The Hub UI state (`docs/HUB.md` §4.1, §9.1). The library lives in the
// host; this module keeps a copy in signals and reloads it after each
// change, because `plinth:hub` calls return at once and do not notify.
import { signal, computed } from "plinth:ui";
import { JSON } from "plinth:core";
import { listApps, listGroups, lastError } from "plinth:hub";

export interface HubCapability {
  name: string;
  risk: string;
  description: string;
  rationale: string;
  decided: boolean;
  allowed: boolean;
  byDefault: boolean;
}

export interface HubApp {
  id: string;
  name: string;
  version: string;
  publisher: string;
  signer: string;
  source: string;
  blocked: boolean;
  groups: string[];
  capabilities: HubCapability[];
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

/** Reads the library and the groups from the host again. */
export function reload(): void {
  const json = listApps();
  if (json === null) {
    loadError.set("The host refused to show the library (" + (lastError() ?? "denied") + ").");
    apps.set([]);
  } else {
    const parsed = JSON.parse<HubApp[]>(json);
    if (parsed === null) {
      loadError.set("The host sent a library that this Hub cannot read.");
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
  const state = app.blocked ? " · Blocked" : "";
  return `${app.version} · ${source}${state}`;
}
