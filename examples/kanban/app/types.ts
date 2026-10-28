// The data types and the fixed lists of the board.

/** One issue. `status` is a column id, `pos` the place in its column. */
export type Issue = {
  id: number;
  title: string;
  description: string;
  kind: string;
  priority: string;
  assignee: string;
  labels: string;
  status: string;
  pos: number;
};

/** The column ids, in board order. */
export const COLUMNS: string[] = ["backlog", "progress", "review", "done"];
/** The column names, in the same order. */
export const COLUMN_NAMES: string[] = ["Backlog", "In Progress", "Review", "Done"];

export function columnIndex(status: string): number {
  const at = COLUMNS.indexOf(status);
  return at < 0 ? 0 : at;
}

export function columnName(status: string): string {
  return COLUMN_NAMES[columnIndex(status)];
}

export function statusOfName(name: string): string {
  const at = COLUMN_NAMES.indexOf(name);
  return COLUMNS[at < 0 ? 0 : at];
}

export function keyOf(i: Issue): string {
  return `KAN-${i.id}`;
}

/** The two-letter initials of a team member. */
export function initials(name: string): string {
  if (name === "Ada") {
    return "AL";
  }
  if (name === "Grace") {
    return "GH";
  }
  if (name === "Alan") {
    return "AT";
  }
  return "?";
}

/** The labels of an issue: the comma separated text, split and trimmed. */
export function labelsOf(i: Issue): string[] {
  const out: string[] = [];
  for (const part of i.labels.split(",")) {
    const label = part.trim();
    if (label !== "") {
      out.push(label);
    }
  }
  return out;
}
