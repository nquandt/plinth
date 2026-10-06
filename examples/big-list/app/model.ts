// Generates 10,000 rows so `rows.tsx` can exercise host-side list
// virtualization (SPEC.md §7.3).
import { signal, computed } from "plinth:ui";

export interface Row {
  id: number;
  title: string;
  subtitle: string;
  on: boolean;
}

const ROW_COUNT = 10_000;

function makeRow(id: number): Row {
  return { id, title: `Row ${id}`, subtitle: `Item number ${id} of ${ROW_COUNT}`, on: false };
}

function makeRows(count: number): Row[] {
  const out: Row[] = [];
  for (let i = 1; i <= count; i++) {
    out.push(makeRow(i));
  }
  return out;
}

export const rows = signal<Row[]>(makeRows(ROW_COUNT));

export const filter = signal("");

export const visibleRows = computed(() => {
  const needle = filter().trim().toLowerCase();
  if (needle === "") {
    return rows();
  }
  return rows().filter((r) => r.title.toLowerCase().includes(needle) || r.subtitle.toLowerCase().includes(needle));
});

export function setRowOn(id: number, on: boolean): void {
  rows.set(rows().map((r) => (r.id === id ? { ...r, on } : r)));
}

export function toggleAll(on: boolean): void {
  rows.set(rows().map((r) => ({ ...r, on })));
}
