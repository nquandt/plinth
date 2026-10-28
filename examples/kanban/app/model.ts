// The state of the board and the changes the user can make. The screens
// import these signals and functions.
//
// `issues` is a copy of the table. A change writes to the database and then
// reads the table again; a move shows at once and writes after it.
import { signal, computed, effect } from "plinth:ui";
import type { ChartPoint } from "plinth:ui";
import { parseNumber } from "plinth:core";
import { openDatabase, allIssues, searchIds, insertIssue, changeIssue, removeIssue, savePlaces, insertMany } from "./storage";
import { COLUMNS, columnIndex, statusOfName } from "./types";
import type { Issue } from "./types";

export const issues = signal<Issue[]>([]);

/** A problem with the database, or "" when there is none. */
export const status = signal("Loading issues...");

// -- Filters ---------------------------------------------------------------

export const search = signal("");
export const assigneeFilter = signal("Anyone");
/** The priorities to show. Empty: all. */
export const priorityFilter = signal<string[]>([]);
/** The ids that the last search found, or null when the box is empty. */
const hits = signal<Set<number> | null>(null);

export function togglePriorityFilter(priority: string): void {
  const now = priorityFilter();
  if (now.includes(priority)) {
    priorityFilter.set(now.filter((p) => p !== priority));
  } else {
    priorityFilter.set([...now, priority]);
  }
}

export function clearFilters(): void {
  search.set("");
  assigneeFilter.set("Anyone");
  priorityFilter.set([]);
}

export const filtering = computed(() => search().trim() !== "" || assigneeFilter() !== "Anyone" || priorityFilter().length > 0);

// Each search has a number. Only the last search sets `hits`, so a slow
// answer to an old text cannot replace a newer one.
let lastSearch = 0;

async function runSearch(text: string): Promise<void> {
  lastSearch += 1;
  const mine = lastSearch;
  try {
    const ids = await searchIds(text);
    if (mine === lastSearch) {
      const found = new Set<number>();
      for (const id of ids) {
        found.add(id);
      }
      hits.set(found);
    }
  } catch (e) {
    status.set(`Search failed: ${(e as Error).message}`);
  }
}

effect(() => {
  const text = search();
  if (text.trim() === "") {
    lastSearch += 1;
    hits.set(null);
  } else {
    runSearch(text);
  }
});

function matches(i: Issue): boolean {
  const found = hits();
  if (found !== null && !found.has(i.id)) {
    return false;
  }
  const who = assigneeFilter();
  if (who !== "Anyone" && i.assignee !== who) {
    return false;
  }
  const levels = priorityFilter();
  return levels.length === 0 || levels.includes(i.priority);
}

function byPlace(a: Issue, b: Issue): number {
  return a.pos !== b.pos ? a.pos - b.pos : a.id - b.id;
}

/** The issues that pass the filters, one array for each column, in order. */
export const board = computed(() => {
  const out: Issue[][] = [[], [], [], []];
  for (const i of issues()) {
    if (matches(i)) {
      out[columnIndex(i.status)].push(i);
    }
  }
  for (const column of out) {
    column.sort(byPlace);
  }
  return out;
});

export const counts = computed(() => {
  const out = [0, 0, 0, 0];
  for (const i of issues()) {
    out[columnIndex(i.status)] += 1;
  }
  return out;
});

export const chartData = computed(() => {
  const n = counts();
  const points: ChartPoint[] = [
    { label: "Backlog", value: n[0] },
    { label: "In Progress", value: n[1] },
    { label: "Review", value: n[2] },
    { label: "Done", value: n[3] },
  ];
  return points;
});

export const summary = computed(() => {
  const n = counts();
  const total = n[0] + n[1] + n[2] + n[3];
  if (total === 0) {
    return "No issues yet";
  }
  return `${total} issues, ${n[3]} done`;
});

export const doneShare = computed(() => {
  const n = counts();
  const total = n[0] + n[1] + n[2] + n[3];
  return total === 0 ? 0 : n[3] / total;
});

/** The cards of a column after the filters, for the column header. */
export function shownCount(column: number): number {
  return board()[column].length;
}

// -- Loading ---------------------------------------------------------------

export async function reload(): Promise<void> {
  issues.set(await allIssues());
}

async function start(): Promise<void> {
  try {
    await openDatabase();
    await reload();
    status.set("");
  } catch (e) {
    status.set(`Issues cannot load: ${(e as Error).message}`);
  }
}

start();

async function apply(change: Promise<void>): Promise<void> {
  try {
    await change;
    await reload();
  } catch (e) {
    status.set(`The change was not saved: ${(e as Error).message}`);
  }
}

// -- Moving ----------------------------------------------------------------

/**
 * Moves an issue to a column, before the issue `beforeId` (-1: to the end).
 * The new order shows at once; the places are written after it.
 */
export function moveIssue(id: number, to: string, beforeId: number): void {
  const all = issues();
  const moving = all.find((i) => i.id === id);
  if (moving === null) {
    return;
  }
  const column = all.filter((i) => i.status === to && i.id !== id).sort(byPlace);
  let at = column.length;
  if (beforeId >= 0) {
    const k = column.findIndex((i) => i.id === beforeId);
    if (k >= 0) {
      at = k;
    }
  }
  const was = all.filter((i) => i.status === to).sort(byPlace).findIndex((i) => i.id === id);
  if (was === at) {
    return;
  }
  column.splice(at, 0, moving);
  const rest = all.filter((i) => i.status !== to && i.id !== id);
  const places: string[][] = [];
  const next: Issue[] = [];
  for (let k = 0; k < column.length; k += 1) {
    const it = column[k];
    if (it.pos !== k + 1 || it.id === id) {
      places.push([to, `${k + 1}`, `${it.id}`]);
    }
    next.push({ ...it, status: to, pos: k + 1 });
  }
  issues.set([...rest, ...next]);
  savePlaces(places).catch((e) => {
    status.set(`The move was not saved: ${e.message}`);
    reload();
  });
}

/** One place up or down in the column (`step` is -1 or 1). */
export function nudgeIssue(id: number, step: number): void {
  const moving = issues().find((i) => i.id === id);
  if (moving === null) {
    return;
  }
  const column = issues().filter((i) => i.status === moving.status).sort(byPlace);
  const at = column.findIndex((i) => i.id === id);
  if (step < 0 && at > 0) {
    moveIssue(id, moving.status, column[at - 1].id);
  } else if (step > 0 && at >= 0 && at < column.length - 1) {
    const after = at + 2 < column.length ? column[at + 2].id : -1;
    moveIssue(id, moving.status, after);
  }
}

/** One column to the left or right (`step` is -1 or 1), to the end of that column. */
export function shiftIssue(id: number, step: number): void {
  const moving = issues().find((i) => i.id === id);
  if (moving === null) {
    return;
  }
  const to = columnIndex(moving.status) + step;
  if (to >= 0 && to < COLUMNS.length) {
    moveIssue(id, COLUMNS[to], -1);
  }
}

/** A card was dropped on a card: before it in its column, or after it when the drop is in the lower half. */
export function dropOnCard(value: string, target: Issue, y: number, cardHeight: number): void {
  const id = parseNumber(value);
  if (id === target.id) {
    return;
  }
  if (y < cardHeight / 2) {
    moveIssue(id, target.status, target.id);
    return;
  }
  const column = issues().filter((i) => i.status === target.status && i.id !== id).sort(byPlace);
  const at = column.findIndex((i) => i.id === target.id);
  moveIssue(id, target.status, at >= 0 && at + 1 < column.length ? column[at + 1].id : -1);
}

/** A card was dropped on the empty part of a column: to its end. */
export function dropOnColumn(value: string, to: string): void {
  moveIssue(parseNumber(value), to, -1);
}

// -- Create, edit, delete --------------------------------------------------

export const editOpen = signal(false);
/** 0: a new issue. */
export const editingId = signal(0);
export const draftTitle = signal("");
export const draftDescription = signal("");
export const draftKind = signal("Task");
export const draftPriority = signal("Medium");
export const draftAssignee = signal("Unassigned");
export const draftLabels = signal("");
const draftStatus = signal("backlog");
export const titleError = signal("");

export function startCreate(column: string): void {
  editingId.set(0);
  draftTitle.set("");
  draftDescription.set("");
  draftKind.set("Task");
  draftPriority.set("Medium");
  draftAssignee.set("Unassigned");
  draftLabels.set("");
  draftStatus.set(column);
  titleError.set("");
  editOpen.set(true);
}

export function startEdit(id: number): void {
  const it = issues().find((i) => i.id === id);
  if (it === null) {
    return;
  }
  editingId.set(id);
  draftTitle.set(it.title);
  draftDescription.set(it.description);
  draftKind.set(it.kind);
  draftPriority.set(it.priority);
  draftAssignee.set(it.assignee === "" ? "Unassigned" : it.assignee);
  draftLabels.set(it.labels);
  draftStatus.set(it.status);
  titleError.set("");
  detailOpen.set(false);
  editOpen.set(true);
}

export function closeEditor(): void {
  editOpen.set(false);
}

export function saveDraft(): void {
  const title = draftTitle().trim();
  if (title === "") {
    titleError.set("Type a title");
    return;
  }
  const assignee = draftAssignee() === "Unassigned" ? "" : draftAssignee();
  const labels = draftLabels().trim();
  if (editingId() === 0) {
    apply(insertIssue(title, draftDescription(), draftKind(), draftPriority(), assignee, labels, draftStatus()));
  } else {
    apply(changeIssue(editingId(), title, draftDescription(), draftKind(), draftPriority(), assignee, labels));
  }
  editOpen.set(false);
}

export function deleteIssue(id: number): void {
  if (detailId() === id) {
    detailOpen.set(false);
  }
  issues.set(issues().filter((i) => i.id !== id));
  apply(removeIssue(id));
}

export function addDemoIssues(count: number): void {
  let top = 0;
  for (const i of issues()) {
    top = i.id > top ? i.id : top;
  }
  apply(insertMany(count, top + 1));
}

// -- The detail sheet and the card menu -----------------------------------

export const detailOpen = signal(false);
export const detailId = signal(0);

const BLANK: Issue = { id: 0, title: "", description: "", kind: "Task", priority: "Medium", assignee: "", labels: "", status: "backlog", pos: 0 };

/** The issue of the detail sheet (a blank one while the sheet is closed). */
export const detail = computed(() => {
  const found = issues().find((i) => i.id === detailId());
  return found === null ? BLANK : found;
});

export function openDetail(id: number): void {
  detailId.set(id);
  detailOpen.set(true);
}

export const menuOpen = signal(false);
export const menuId = signal(0);

export function openMenu(id: number): void {
  menuId.set(id);
  menuOpen.set(true);
}

export function moveByName(id: number, name: string): void {
  moveIssue(id, statusOfName(name), -1);
}

/** The keys of a focused card (docs/ui.md "Keys"). */
export function cardKey(i: Issue, key: string): void {
  if (key === "e") {
    startEdit(i.id);
  } else if (key === "Delete") {
    deleteIssue(i.id);
  } else if (key === "u") {
    nudgeIssue(i.id, -1);
  } else if (key === "d") {
    nudgeIssue(i.id, 1);
  } else if (key === "ArrowLeft") {
    shiftIssue(i.id, -1);
  } else if (key === "ArrowRight") {
    shiftIssue(i.id, 1);
  }
}
