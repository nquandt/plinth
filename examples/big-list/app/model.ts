// Generates 10,000 rows so `rows.tsx` can exercise host-side list
// virtualization (SPEC.md §7.3).
import { signal, computed } from "plinth:ui";

export interface Row {
  id: number;
  title: string;
  subtitle?: string;
  badge: boolean;
  on: boolean;
}

const ROW_COUNT = 10_000;

// Mixed row shapes exercise variable-height virtualization (SPEC.md §7.3):
// every third row has no subtitle (shorter), and every tenth row carries a
// trailing Badge (taller than a Toggle-only row).
function makeRow(id: number): Row {
  return {
    id,
    title: `Row ${id}`,
    subtitle: id % 3 === 0 ? undefined : `Item number ${id} of ${ROW_COUNT}`,
    badge: id % 10 === 0,
    on: false,
  };
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
  return rows().filter(
    (r) => r.title.toLowerCase().includes(needle) || (r.subtitle ?? "").toLowerCase().includes(needle),
  );
});

export function setRowOn(id: number, on: boolean): void {
  rows.set(rows().map((r) => (r.id === id ? { ...r, on } : r)));
}

export function toggleAll(on: boolean): void {
  rows.set(rows().map((r) => ({ ...r, on })));
}
