// Tests of the task list: `npm test` (or `plinth test`) runs them. Each
// test starts a new app with an empty store.
import { test, renderApp, expect } from "plinth:test";

test("adds a task", async () => {
  const s = await renderApp();
  expect(s.byText("0 of 0 left")).toExist();
  await s.type("Title", "Buy milk");
  await s.press("Add task");
  expect(s.row("Buy milk")).toExist();
  expect(s.byText("1 of 1 left")).toExist();
});

test("marks a task as done and opens it", async () => {
  const s = await renderApp();
  await s.type("Title", "Buy milk");
  await s.press("Add task");
  await s.row("Buy milk").get("Done").press();
  expect(s.byText("0 of 1 left")).toExist();
  await s.row("Buy milk").press();
  expect(s.get("Status")).toExist();
  expect(s.byText("Done")).toExist();
});
