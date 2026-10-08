import { test, render, renderApp, expect } from "plinth:test";
import Tasks from "./tasks";

test("starts with three open tasks", async () => {
  const s = await render(<Tasks />);
  expect(s.byRole("row")).toHaveCount(3);
  expect(s.byText("3 of 3 remaining")).toExist();
  expect(s.get("Add task")).toBeDisabled();
});

test("adds a task", async () => {
  const s = await render(<Tasks />);
  await s.type("Task", "Buy milk");
  await s.press("Add task");
  expect(s.row("Buy milk")).toExist();
  expect(s.row("Buy milk")).toHaveText("Buy milk");
  expect(s.byText("4 of 4 remaining")).toExist();
  expect(s.get("Task")).toHaveText("");
});

test("adds a task with the Enter key", async () => {
  const s = await render(<Tasks />);
  await s.type("Task", "Call Sam");
  await s.submit("Task");
  expect(s.row("Call Sam")).toExist();
});

test("does not add an empty task", async () => {
  const s = await render(<Tasks />);
  await s.type("Task", "   ");
  expect(s.get("Add task")).toBeDisabled();
  await s.submit("Task");
  expect(s.byRole("row")).toHaveCount(3);
});

test("marks a task as done", async () => {
  const s = await render(<Tasks />);
  await s.row("Read SPEC.md").get("Done").toggle();
  expect(s.row("Read SPEC.md").get("Done")).toBeChecked();
  expect(s.row("Build the M0 host").get("Done")).not.toBeChecked();
  expect(s.byText("2 of 3 remaining")).toExist();
});

test("deletes a task", async () => {
  const s = await render(<Tasks />);
  await s.row("Build the M0 host").get("Delete").press();
  expect(s.row("Build the M0 host")).not.toExist();
  expect(s.byRole("row")).toHaveCount(2);
});

test("moves a task down with Alt+Down", async () => {
  const s = await render(<Tasks />);
  await s.row("Read SPEC.md").moveDown();
  const text = await s.text();
  expect(text.indexOf("Build the M0 host") < text.indexOf("Read SPEC.md")).toBe(true);
});

test("hides completed tasks", async () => {
  const s = await renderApp();
  await s.row("Write the compiler").get("Done").toggle();
  await s.showScreen("Settings");
  await s.toggle("Show completed tasks");
  expect(s.get("Clear 1 completed task")).toExist();
  await s.showScreen("Tasks");
  expect(s.row("Write the compiler")).not.toExist();
  expect(s.byRole("row")).toHaveCount(2);
  expect(s.byText("2 of 3 remaining")).toExist();
});
