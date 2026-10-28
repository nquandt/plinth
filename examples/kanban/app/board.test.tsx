import { test, render, settle, expect } from "plinth:test";
import Board from "./board";
import { issues, board, reload, titleError, moveIssue, nudgeIssue, shiftIssue, dropOnCard, dropOnColumn, cardKey, deleteIssue, search, summary, addDemoIssues, DEMO_LIMIT } from "./model";
import { allIssues, matchText } from "./storage";
import type { Issue } from "./types";
import { columnIndex } from "./types";

/** The ids of a column, top to bottom, as text. */
function ids(column: number): string {
  return board()[column].map((i) => `${i.id}`).join(",");
}

/** The rows of the table. The test waits for the database with `settle`. */
async function rows(): Promise<Issue[]> {
  const read = allIssues();
  await settle();
  return await read;
}

async function readAgain(): Promise<void> {
  const done = reload();
  await settle();
  await done;
}

async function start() {
  const s = await render(<Board />);
  await settle();
  return s;
}

test("starts with the first issues in their columns", async () => {
  const s = await start();
  expect(s.get("KAN-7: Export issues to CSV")).toExist();
  expect(s.get("Backlog issues").get("KAN-7: Export issues to CSV")).toExist();
  expect(s.get("In Progress issues").get("KAN-3: Login fails on a slow network")).toExist();
  expect(s.get("Review issues").get("KAN-5: Write the release notes")).toExist();
  expect(s.get("Done issues").get("KAN-1: Set up the project")).toExist();
  expect(s.byText("10 issues, 2 done")).toExist();
  expect(ids(0)).toBe("7,8,9,10");
});

test("creates an issue in a dialog", async () => {
  const s = await start();
  await s.press("Add issue to Review");
  await s.type("Title", "Write the docs");
  await s.select("Priority", "Urgent");
  await s.select("Assignee", "Grace");
  await s.type("Labels", "docs, writing");
  await s.press("Save");
  await settle();
  expect(s.get("Review issues").get("KAN-11: Write the docs")).toExist();
  expect(s.byText("11 issues, 2 done")).toExist();
  expect(ids(2)).toBe("5,6,11");
  const saved = (await rows()).find((i) => i.id === 11);
  expect(saved === null).toBe(false);
  if (saved !== null) {
    expect(saved.priority).toBe("Urgent");
    expect(saved.assignee).toBe("Grace");
    expect(saved.labels).toBe("docs, writing");
  }
});

test("does not create an issue with no title", async () => {
  const s = await start();
  await s.press("Add issue to Backlog");
  await s.press("Save");
  await settle();
  expect(titleError()).toBe("Type a title");
  expect(s.get("Title")).toExist();
  expect(s.byText("10 issues, 2 done")).toExist();
});

test("edits an issue from its detail sheet", async () => {
  const s = await start();
  await s.press("KAN-9: Upgrade the build tools");
  expect(s.byText("No description.")).toExist();
  await s.press("Edit issue");
  await s.type("Title", "Upgrade the toolchain");
  await s.type("Description", "Rust and Node.");
  await s.press("Save");
  await settle();
  expect(s.get("KAN-9: Upgrade the toolchain")).toExist();
  expect(s.get("KAN-9: Upgrade the build tools")).not.toExist();
  const saved = (await rows()).find((i) => i.id === 9);
  expect(saved === null ? "" : saved.description).toBe("Rust and Node.");
});

test("moves an issue to another column from its sheet", async () => {
  const s = await start();
  await s.press("KAN-7: Export issues to CSV");
  await s.press("Move to Review");
  await settle();
  expect(s.get("Review issues").get("KAN-7: Export issues to CSV")).toExist();
  expect(ids(0)).toBe("8,9,10");
  expect(ids(2)).toBe("5,6,7");
  expect(columnIndex((await rows()).filter((i) => i.id === 7)[0].status)).toBe(2);
});

test("a drop on a column puts the issue at its end", async () => {
  const s = await start();
  dropOnColumn("8", "done");
  await settle();
  expect(ids(3)).toBe("1,2,8");
  expect(ids(0)).toBe("7,9,10");
  expect(s.get("Done issues").get("KAN-8: Keyboard shortcuts for cards")).toExist();
});

test("a drop on a card puts the issue before it or after it", async () => {
  await start();
  // The upper half of KAN-8: before it.
  dropOnCard("10", (await rows())[7], 3, 33);
  expect(ids(0)).toBe("7,10,8,9");
  // The lower half of KAN-8: after it.
  dropOnCard("7", board()[0][2], 28, 33);
  expect(ids(0)).toBe("10,8,7,9");
  // A drop on itself does nothing.
  dropOnCard("8", board()[0][1], 3, 33);
  expect(ids(0)).toBe("10,8,7,9");
  // The order is in the database too.
  await settle();
  const rows = (await rows()).filter((i) => i.status === "backlog");
  expect(rows.map((i) => `${i.id}`).join(",")).toBe("10,8,7,9");
});

test("the keys move a card", async () => {
  await start();
  const second = board()[0][1];
  cardKey(second, "u");
  expect(ids(0)).toBe("8,7,9,10");
  cardKey(board()[0][0], "d");
  expect(ids(0)).toBe("7,8,9,10");
  cardKey(board()[0][3], "ArrowRight");
  expect(ids(1)).toBe("3,4,10");
  cardKey(board()[1][2], "ArrowLeft");
  expect(ids(0)).toBe("7,8,9,10");
  // At the edges nothing happens.
  shiftIssue(7, -1);
  nudgeIssue(7, -1);
  expect(ids(0)).toBe("7,8,9,10");
  moveIssue(10, "backlog", -1);
  expect(ids(0)).toBe("7,8,9,10");
  cardKey(board()[0][0], "Delete");
  expect(ids(0)).toBe("8,9,10");
  await settle();
  expect((await rows()).length).toBe(9);
});

test("filters by text, assignee and priority", async () => {
  const s = await start();
  await s.type("Search", "login");
  await settle();
  expect(s.get("KAN-3: Login fails on a slow network")).toExist();
  expect(s.get("KAN-2: Design the login screen")).toExist();
  expect(s.get("KAN-7: Export issues to CSV")).not.toExist();
  // A prefix finds a word, and a quote or an operator is only text.
  await s.type("Search", "perf");
  await settle();
  expect(s.get("KAN-10: Search is slow with many issues")).toExist();
  expect(s.get("KAN-3: Login fails on a slow network")).not.toExist();
  await s.type("Search", "\"AND NEAR(");
  await settle();
  expect(s.get("KAN-10: Search is slow with many issues")).not.toExist();
  await s.type("Search", "");
  await settle();
  expect(s.get("KAN-7: Export issues to CSV")).toExist();

  await s.select("Assigned to", "Ada");
  expect(s.get("KAN-1: Set up the project")).toExist();
  expect(s.get("KAN-3: Login fails on a slow network")).not.toExist();
  expect(s.byText("1 of 4")).toExist();
  await s.press("Show High priority");
  expect(s.get("KAN-1: Set up the project")).toExist();
  expect(s.get("KAN-9: Upgrade the build tools")).not.toExist();
  await s.byRole("pressable", "Clear filters").press();
  expect(s.get("KAN-9: Upgrade the build tools")).toExist();
  expect(s.byRole("pressable", "Clear filters")).not.toExist();
});

test("deletes an issue from its sheet", async () => {
  const s = await start();
  await s.press("KAN-8: Keyboard shortcuts for cards");
  await s.press("Delete issue");
  await settle();
  expect(s.get("KAN-8: Keyboard shortcuts for cards")).not.toExist();
  expect(s.byText("9 issues, 2 done")).toExist();
  expect((await rows()).length).toBe(9);
});

test("keeps the issues in the database", async () => {
  const s = await start();
  await s.press("Add issue to Done");
  await s.type("Title", "Saved for later");
  await s.press("Save");
  await settle();
  moveIssue(11, "progress", 3);
  await settle();
  // Forget the copy in memory and read the table again.
  issues.set([]);
  expect(s.byText("No issues yet")).toExist();
  await readAgain();
  expect(s.get("In Progress issues").get("KAN-11: Saved for later")).toExist();
  expect(ids(1)).toBe("11,3,4");
  // An expectation about the screen is checked at the next await: wait before the next change.
  await settle();
  deleteIssue(11);
  await settle();
  await readAgain();
  expect(s.get("KAN-11: Saved for later")).not.toExist();
});

test("the search text is a quoted prefix for each word", async () => {
  expect(matchText("  login   fails ")).toBe("\"login\"* \"fails\"*");
  expect(matchText("say \"hi\"")).toBe("\"say\"* \"\"\"hi\"\"\"*");
  expect(matchText("   ")).toBe("");
  expect(summary()).toBe("No issues yet");
  search.set("x");
  expect(search()).toBe("x");
});

test("the demo action stops at the limit, also when it is pressed again at once", async () => {
  const s = await start();
  // Three presses before the first one is done: the later ones do nothing.
  addDemoIssues(500);
  addDemoIssues(500);
  addDemoIssues(500);
  await settle();
  expect((await rows()).length).toBe(510);
  addDemoIssues(500);
  await settle();
  expect((await rows()).length).toBe(DEMO_LIMIT);
  await s.press("Add 500 demo issues");
  await settle();
  expect((await rows()).length).toBe(DEMO_LIMIT);
});
