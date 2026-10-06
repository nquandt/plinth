// Pure mapping from `plinth check --json` output to editor-agnostic
// diagnostics. Kept free of the `vscode` module so it can be unit-tested
// with plain node (test/mapDiagnostics.test.js).
//
// `plinth check --json` prints a JSON array of objects shaped like
// (crates/plinth-compiler/src/diag.rs `to_json`):
//   { file, line, column, start, end, severity: "error"|"warning",
//     code, message, help: string | null }
// `line`/`column` are 1-based and point at the start of the span. There is
// no end line/column, so a diagnostic that spans more than one line is
// clamped to the rest of its start line (good enough for a squiggle).

export interface PlinthDiagnosticJson {
  file: string;
  line: number;
  column: number;
  start: number;
  end: number;
  severity: "error" | "warning";
  code: string;
  message: string;
  help: string | null;
}

/** A 0-based, editor-agnostic position. */
export interface Pos {
  line: number;
  character: number;
}

/** A 0-based, editor-agnostic diagnostic, grouped by file. */
export interface MappedDiagnostic {
  file: string;
  range: { start: Pos; end: Pos };
  severity: "error" | "warning";
  code: string;
  message: string;
  help: string | null;
}

/** Parses `plinth check --json` stdout. Throws on malformed JSON. */
export function parsePlinthJson(text: string): PlinthDiagnosticJson[] {
  const parsed = JSON.parse(text);
  if (!Array.isArray(parsed)) {
    throw new Error("expected a JSON array of diagnostics");
  }
  return parsed as PlinthDiagnosticJson[];
}

/** Maps parsed diagnostics to 0-based ranges an editor can use directly. */
export function mapDiagnostics(diags: PlinthDiagnosticJson[]): MappedDiagnostic[] {
  return diags.map((d) => {
    const line = Math.max(0, d.line - 1);
    const startChar = Math.max(0, d.column - 1);
    const width = Math.max(1, d.end - d.start);
    return {
      file: d.file,
      range: {
        start: { line, character: startChar },
        end: { line, character: startChar + width },
      },
      severity: d.severity,
      code: d.code,
      message: d.message,
      help: d.help,
    };
  });
}

/** Groups mapped diagnostics by (absolute) file path. */
export function groupByFile(diags: MappedDiagnostic[]): Map<string, MappedDiagnostic[]> {
  const out = new Map<string, MappedDiagnostic[]>();
  for (const d of diags) {
    const list = out.get(d.file);
    if (list) {
      list.push(d);
    } else {
      out.set(d.file, [d]);
    }
  }
  return out;
}

/** The status-bar summary text: "Plinth: ok" or "Plinth: N errors". */
export function statusText(diags: PlinthDiagnosticJson[]): string {
  const errors = diags.filter((d) => d.severity === "error").length;
  if (errors === 0) {
    return "Plinth: ok";
  }
  return errors === 1 ? "Plinth: 1 error" : `Plinth: ${errors} errors`;
}
