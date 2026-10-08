import { test, renderApp, expect } from "plinth:test";

test("counts up and down", async () => {
  const s = await renderApp();
  expect(s.byRole("heading")).toHaveText("0");
  await s.press("Increment");
  await s.press("Increment");
  expect(s.byRole("heading")).toHaveText("2");
  expect(s.byText("even")).toExist();
  await s.press("Decrement");
  expect(s.byRole("heading")).toHaveText("1");
  expect(s.byText("odd")).toExist();
  await s.press("Reset");
  expect(s.byRole("heading")).toHaveText("0");
});
