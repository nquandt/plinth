// The issues in the app's SQL database (`store.sql`, docs/SQL.md).
//
// `issues` holds the issues. `status` is the column, `pos` the place in the
// column (1, 2, 3 ...; gaps are fine). The full-text index `issues_fts`
// (SQLite FTS5, as in examples/notes) reads its text from `issues`; the
// triggers keep it in step.
import { sql } from "plinth:store";
import type { Issue } from "./types";

// Migration steps are append-only: never change or remove a step.
const SCHEMA = [
  "CREATE TABLE issues (id INTEGER PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL, kind TEXT NOT NULL, priority TEXT NOT NULL, assignee TEXT NOT NULL, labels TEXT NOT NULL, status TEXT NOT NULL, pos INTEGER NOT NULL)",
  "CREATE INDEX issues_status ON issues (status, pos)",
  "CREATE VIRTUAL TABLE issues_fts USING fts5(title, description, labels, content='issues', content_rowid='id')",
  "CREATE TRIGGER issues_ai AFTER INSERT ON issues BEGIN INSERT INTO issues_fts (rowid, title, description, labels) VALUES (new.id, new.title, new.description, new.labels); END",
  "CREATE TRIGGER issues_ad AFTER DELETE ON issues BEGIN INSERT INTO issues_fts (issues_fts, rowid, title, description, labels) VALUES ('delete', old.id, old.title, old.description, old.labels); END",
  "CREATE TRIGGER issues_au AFTER UPDATE ON issues BEGIN INSERT INTO issues_fts (issues_fts, rowid, title, description, labels) VALUES ('delete', old.id, old.title, old.description, old.labels); INSERT INTO issues_fts (rowid, title, description, labels) VALUES (new.id, new.title, new.description, new.labels); END",
];

const INSERT =
  "INSERT INTO issues (title, description, kind, priority, assignee, labels, status, pos) VALUES (?, ?, ?, ?, ?, ?, ?, (SELECT coalesce(max(pos), 0) + 1 FROM issues WHERE status = ?))";
const UPDATE = "UPDATE issues SET title = ?, description = ?, kind = ?, priority = ?, assignee = ?, labels = ? WHERE id = ?";
const SET_PLACE = "UPDATE issues SET status = ?, pos = ? WHERE id = ?";
const DELETE = "DELETE FROM issues WHERE id = ?";
const ALL = "SELECT id, title, description, kind, priority, assignee, labels, status, pos FROM issues ORDER BY pos, id";
const SEARCH = "SELECT rowid AS n FROM issues_fts WHERE issues_fts MATCH ?";

/** The first issues of a new database: title, kind, priority, assignee, labels, status. */
const FIRST: string[][] = [
  ["Set up the project", "Task", "High", "Ada", "setup", "done"],
  ["Design the login screen", "Story", "Medium", "Grace", "design,ux", "done"],
  ["Login fails on a slow network", "Bug", "Urgent", "Alan", "auth,network", "progress"],
  ["Add a dark theme", "Story", "Medium", "Grace", "design", "progress"],
  ["Write the release notes", "Task", "Low", "", "docs", "review"],
  ["Crash when the title is empty", "Bug", "High", "Ada", "editor", "review"],
  ["Export issues to CSV", "Story", "Low", "", "export", "backlog"],
  ["Keyboard shortcuts for cards", "Story", "Medium", "Alan", "a11y,ux", "backlog"],
  ["Upgrade the build tools", "Task", "Low", "Ada", "build", "backlog"],
  ["Search is slow with many issues", "Bug", "High", "", "perf,search", "backlog"],
];

/** Makes or updates the tables. A new database gets the first issues. */
export async function openDatabase(): Promise<void> {
  const before = await sql.query<{ user_version: number }>("PRAGMA user_version");
  await sql.migrate(SCHEMA);
  if (before[0].user_version === 0) {
    const steps: { sql: string; params: string[] }[] = [];
    for (const row of FIRST) {
      steps.push({ sql: INSERT, params: [row[0], "", row[1], row[2], row[3], row[4], row[5], row[5]] });
    }
    await sql.batch(steps);
  }
}

export function allIssues(): Promise<Issue[]> {
  return sql.query<Issue>(ALL);
}

/** The ids of the issues that have each word of `text` (or a word that starts with it). */
export async function searchIds(text: string): Promise<number[]> {
  const rows = await sql.query<{ n: number }>(SEARCH, [matchText(text)]);
  return rows.map((r) => r.n);
}

/** The FTS5 query for the text of the search box: each word is a quoted prefix, so no text is an operator. */
export function matchText(text: string): string {
  const words: string[] = [];
  for (const word of text.trim().split(/\s+/)) {
    if (word !== "") {
      words.push(`"${word.replaceAll("\"", "\"\"")}"*`);
    }
  }
  return words.join(" ");
}

export async function insertIssue(title: string, description: string, kind: string, priority: string, assignee: string, labels: string, status: string): Promise<void> {
  await sql.exec(INSERT, [title, description, kind, priority, assignee, labels, status, status]);
}

export async function changeIssue(id: number, title: string, description: string, kind: string, priority: string, assignee: string, labels: string): Promise<void> {
  await sql.exec(UPDATE, [title, description, kind, priority, assignee, labels, id]);
}

export async function removeIssue(id: number): Promise<void> {
  await sql.exec(DELETE, [id]);
}

/** Writes the status and the place of some issues in one transaction. Each step: status, pos, id (as text). */
export function savePlaces(places: string[][]): Promise<void> {
  const steps: { sql: string; params: string[] }[] = [];
  for (const p of places) {
    steps.push({ sql: SET_PLACE, params: [p[0], p[1], p[2]] });
  }
  return sql.batch(steps);
}

/** Adds many issues in one transaction (the "demo issues" action). */
export function insertMany(count: number, from: number): Promise<void> {
  const steps: { sql: string; params: string[] }[] = [];
  const kinds = ["Task", "Bug", "Story"];
  const priorities = ["Low", "Medium", "High", "Urgent"];
  const people = ["Ada", "Grace", "Alan", ""];
  const statuses = ["backlog", "progress", "review", "done"];
  for (let n = 0; n < count; n += 1) {
    const k = from + n;
    const status = statuses[k % 4];
    steps.push({
      sql: INSERT,
      params: [`Demo issue number ${k}`, "", kinds[k % 3], priorities[(k * 7) % 4], people[(k * 5) % 4], "demo", status, status],
    });
  }
  return sql.batch(steps);
}
