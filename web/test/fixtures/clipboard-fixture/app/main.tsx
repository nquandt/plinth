// A fixture app for web/test/run-clipboard-dialog.mjs: `plinth:clipboard`
// and `plinth:dialog` on the web host. Each button sets the text to
// "done:<result>".
import { app, Screen, Text, Button, signal } from "plinth:ui";
import { writeText, readText, lastError } from "plinth:clipboard";
import { alert, confirm, prompt } from "plinth:dialog";

const result = signal("waiting");

function writeThenRead(): void {
  writeText("héllo 世界 \u{1F44B}");
  const v = readText();
  result.set("done:" + (v === null ? "null" : v));
}

function readOnly(): void {
  const v = readText();
  result.set("done:" + (v === null ? "null" : v));
}

function writeThenReadWithError(): void {
  writeText("x");
  const v = readText();
  const err = lastError();
  result.set("done:" + (v === null ? "null" : v) + "/" + (err === null ? "none" : err));
}

async function doAlert(): Promise<void> {
  await alert("hello");
  result.set("done:alerted");
}

async function doConfirmTrue(): Promise<void> {
  const ok = await confirm("ok?");
  result.set("done:" + ok);
}

async function doConfirmFalse(): Promise<void> {
  const ok = await confirm("ok?");
  result.set("done:" + ok);
}

async function doPromptText(): Promise<void> {
  const v = await prompt("name?");
  result.set("done:" + (v === null ? "null" : v));
}

async function doPromptCancel(): Promise<void> {
  const v = await prompt("name?");
  result.set("done:" + (v === null ? "null" : v));
}

function Home() {
  return (
    <Screen title="Clipboard Fixture">
      <Button label="write-then-read" onPress={writeThenRead} />
      <Button label="read-only" onPress={readOnly} />
      <Button label="write-then-read-with-error" onPress={writeThenReadWithError} />
      <Button label="alert" onPress={doAlert} />
      <Button label="confirm-true" onPress={doConfirmTrue} />
      <Button label="confirm-false" onPress={doConfirmFalse} />
      <Button label="prompt-text" onPress={doPromptText} />
      <Button label="prompt-cancel" onPress={doPromptCancel} />
      <Text>{result()}</Text>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Home", component: Home } } });
