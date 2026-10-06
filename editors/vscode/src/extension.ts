// Minimal Plinth extension: on open/save of a .ts/.tsx file inside a
// project (a folder with plinth.toml, searched upward from the file), run
// `plinth check --json <project dir>` and publish diagnostics.
import * as cp from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import * as vscode from "vscode";
import { groupByFile, mapDiagnostics, parsePlinthJson, statusText, type PlinthDiagnosticJson } from "./mapDiagnostics";

let diagnostics: vscode.DiagnosticCollection;
let status: vscode.StatusBarItem;

export function activate(context: vscode.ExtensionContext): void {
  diagnostics = vscode.languages.createDiagnosticCollection("plinth");
  status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 0);
  status.text = "Plinth";
  status.show();
  context.subscriptions.push(diagnostics, status);

  const run = (doc: vscode.TextDocument) => checkDocument(doc);

  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument(run),
    vscode.workspace.onDidSaveTextDocument(run),
  );

  for (const doc of vscode.workspace.textDocuments) {
    run(doc);
  }
}

export function deactivate(): void {
  diagnostics?.dispose();
  status?.dispose();
}

function isPlinthSource(doc: vscode.TextDocument): boolean {
  return doc.uri.scheme === "file" && (doc.fileName.endsWith(".ts") || doc.fileName.endsWith(".tsx"));
}

/** Walks upward from `file` looking for the nearest `plinth.toml`. */
function findProjectDir(file: string): string | null {
  let dir = path.dirname(file);
  while (true) {
    if (fs.existsSync(path.join(dir, "plinth.toml"))) {
      return dir;
    }
    const parent = path.dirname(dir);
    if (parent === dir) {
      return null;
    }
    dir = parent;
  }
}

function plinthBinary(projectDir: string): string {
  const configured = vscode.workspace.getConfiguration("plinth").get<string>("path");
  if (configured) {
    return configured;
  }
  const exe = process.platform === "win32" ? "plinth.exe" : "plinth";
  const local = path.join(projectDir, "node_modules", ".bin", exe);
  if (fs.existsSync(local)) {
    return local;
  }
  return "plinth";
}

function checkDocument(doc: vscode.TextDocument): void {
  if (!isPlinthSource(doc)) {
    return;
  }
  const projectDir = findProjectDir(doc.fileName);
  if (!projectDir) {
    return;
  }
  const bin = plinthBinary(projectDir);
  cp.exec(`"${bin}" check --json "${projectDir}"`, { cwd: projectDir, maxBuffer: 10 * 1024 * 1024 }, (_err, stdout) => {
    let parsed: PlinthDiagnosticJson[];
    try {
      parsed = parsePlinthJson(stdout.trim() || "[]");
    } catch {
      return; // not JSON: the binary is missing or failed before printing diagnostics
    }
    publish(projectDir, parsed);
  });
}

function publish(projectDir: string, parsed: PlinthDiagnosticJson[]): void {
  diagnostics.clear();
  const mapped = mapDiagnostics(parsed);
  for (const [file, group] of groupByFile(mapped)) {
    const abs = path.isAbsolute(file) ? file : path.join(projectDir, file);
    const uri = vscode.Uri.file(abs);
    const vsDiags = group.map((d) => {
      const range = new vscode.Range(d.range.start.line, d.range.start.character, d.range.end.line, d.range.end.character);
      const severity = d.severity === "error" ? vscode.DiagnosticSeverity.Error : vscode.DiagnosticSeverity.Warning;
      const diag = new vscode.Diagnostic(range, d.message, severity);
      diag.source = "plinth";
      diag.code = d.code;
      if (d.help) {
        diag.relatedInformation = [new vscode.DiagnosticRelatedInformation(new vscode.Location(uri, range), d.help)];
      }
      return diag;
    });
    diagnostics.set(uri, vsDiags);
  }
  status.text = statusText(parsed);
}
