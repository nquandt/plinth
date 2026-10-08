// A fixture app for web/test/run-a11y.mjs: `store.sql` (core 1.13,
// docs/SQL.md) in a browser, where the page runs SQLite with OPFS. Each
// button sets the text to "done:<result>".
import { app, Screen, Text, Button, signal } from "plinth:ui";
import { sql } from "plinth:store";

const result = signal("waiting");

interface Note { id: number; title: string; done: boolean }
const SCHEMA = [
  "CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0)",
  "CREATE VIRTUAL TABLE search USING fts5(title)",
];

async function add(): Promise<void> {
  try {
    await sql.migrate(SCHEMA);
    await sql.batch([
      { sql: "INSERT INTO notes (title, done) VALUES (?, ?)", params: ["Buy milk", false] },
      { sql: "INSERT INTO search (title) VALUES (?)", params: ["Buy milk"] },
    ]);
    const rows = await sql.query<Note>("SELECT id, title, done FROM notes ORDER BY id");
    result.set("done:rows:" + rows.length);
  } catch (e) {
    result.set("done:err:" + (e as Error).message);
  }
}

async function count(): Promise<void> {
  try {
    const v = await sql.migrate(SCHEMA);
    const rows = await sql.query<Note>("SELECT id, title, done FROM notes ORDER BY id");
    const hits = await sql.query<{ title: string }>("SELECT title FROM search WHERE search MATCH ?", ["milk"]);
    result.set("done:v" + v + ":rows:" + rows.length + ":hits:" + hits.length);
  } catch (e) {
    result.set("done:err:" + (e as Error).message);
  }
}

async function refused(): Promise<void> {
  let r = "";
  try { await sql.exec("ATTACH DATABASE 'x.db' AS x"); r = r + "ok;"; } catch (e) { r = r + (e as Error).message + ";"; }
  try { await sql.exec("PRAGMA journal_mode = OFF"); r = r + "ok;"; } catch (e) { r = r + (e as Error).message + ";"; }
  try {
    await sql.query<{ n: number }>("WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) AS n FROM c");
    r = r + "ok";
  } catch (e) {
    r = r + (e as Error).message;
  }
  result.set("done:" + r);
}

function Home() {
  return (
    <Screen title="Home">
      <Text>{result()}</Text>
      <Button label="add" onPress={() => { add(); }} />
      <Button label="count" onPress={() => { count(); }} />
      <Button label="refused" onPress={() => { refused(); }} />
    </Screen>
  );
}

export default app({ screens: { home: { title: "Home", component: Home } } });
