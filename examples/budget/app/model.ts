// The app state. Screens import these signals and functions.
import { signal, computed, effect } from "plinth:ui";
import { Math } from "plinth:core";
import { parseDate, dateParts } from "plinth:time";
import { loadTransactions, saveTransactions } from "./storage";
import { categories } from "./types";
import type { Category, Transaction } from "./types";

let nextId = 1;

function makeTransaction(date: string, amount: number, category: Category, note: string): Transaction {
  const t: Transaction = { id: nextId, date, amount, category, note };
  nextId += 1;
  return t;
}

function defaultTransactions(): Transaction[] {
  return [
    makeTransaction("2026-10-01", 2500, "Salary", "Paycheck"),
    makeTransaction("2026-10-02", -120.5, "Groceries", "Weekly shop"),
    makeTransaction("2026-10-03", -900, "Rent", "October rent"),
    makeTransaction("2026-10-04", -45, "Transport", "Gas"),
    makeTransaction("2026-10-05", -30, "Fun", "Movie night"),
  ];
}

// Saved transactions (SPEC.md §8.5, `store.kv`) take over `nextId` so new
// transactions do not collide with saved ids.
function startingTransactions(): Transaction[] {
  const saved = loadTransactions();
  if (saved === null) {
    return defaultTransactions();
  }
  for (const t of saved) {
    if (t.id >= nextId) {
      nextId = t.id + 1;
    }
  }
  return saved;
}

export const transactions = signal<Transaction[]>(startingTransactions());

// Persists every change. `effect` reruns whenever `transactions()` changes.
effect(() => {
  saveTransactions(transactions());
});

export const filterCategory = signal<string>("All");
export const search = signal("");

export const filteredTransactions = computed(() => {
  const cat = filterCategory();
  const q = search().trim().toLowerCase();
  return transactions().filter((t) => {
    if (cat !== "All" && t.category !== cat) {
      return false;
    }
    if (q !== "" && !t.note.toLowerCase().includes(q) && !t.category.toLowerCase().includes(q)) {
      return false;
    }
    return true;
  });
});

export const income = computed(() => {
  let total = 0;
  for (const t of transactions()) {
    if (t.amount > 0) {
      total += t.amount;
    }
  }
  return total;
});

export const spending = computed(() => {
  let total = 0;
  for (const t of transactions()) {
    if (t.amount < 0) {
      total += t.amount;
    }
  }
  return total;
});

export const balance = computed(() => income() + spending());

/** Totals per category, as a `Map<Category, number>` (negative for net
 * spending, positive for net income in that category). */
export const totalsByCategory = computed(() => {
  const totals = new Map<Category, number>();
  for (const c of categories) {
    totals.set(c, 0);
  }
  for (const t of transactions()) {
    const current = totals.get(t.category) ?? 0;
    totals.set(t.category, current + t.amount);
  }
  return totals;
});

/** The largest absolute category total, for scaling progress bars. Returns
 * at least 1 to avoid a division by zero. */
export const maxCategoryMagnitude = computed(() => {
  const totals = totalsByCategory();
  let max = 1;
  for (const c of categories) {
    const v = Math.abs(totals.get(c) ?? 0);
    if (v > max) {
      max = v;
    }
  }
  return max;
});

/** `"YYYY-MM"` for a transaction's `date` (parsed with `plinth:time`'s
 * `parseDate`/`dateParts`, docs/GAPS.md gap #5), used to group by month.
 * An unparseable date (should not happen; `DatePicker` always stores an
 * ISO date) groups under `"Unknown"` rather than failing. */
function monthKey(date: string): string {
  const ms = parseDate(date);
  if (ms === null) {
    return "Unknown";
  }
  const p = dateParts(ms, true);
  const month = p.month < 10 ? `0${p.month}` : `${p.month}`;
  return `${p.year}-${month}`;
}

export interface MonthTotal {
  month: string;
  total: number;
}

/** Net total per month, in the order months first appear among
 * `transactions()`. A plain array, not a `Map`: `Map` has no `.keys()`/
 * for…of in `std/lib.d.ts` (`tsc` needs it declared there, which the
 * host's special-cased `for…of` over `Map` is not), so this groups with
 * a linear `find`, the same pattern `totalsByCategory` would use without
 * the fixed `categories` list. */
export const monthlyTotals = computed(() => {
  const result: MonthTotal[] = [];
  for (const t of transactions()) {
    const key = monthKey(t.date);
    const existing = result.find((r) => r.month === key);
    if (existing === null) {
      result.push({ month: key, total: t.amount });
    } else {
      existing.total += t.amount;
    }
  }
  return result;
});

/** `monthlyTotals()` as `"YYYY-MM: total"` lines, ready to render. */
export const monthlyTotalLines = computed(() => monthlyTotals().map((m) => `${m.month}: ${m.total.toFixed(2)}`));

export const selected = signal<number | null>(null);

export function selectedTransaction(): Transaction | null {
  const id = selected();
  if (id === null) {
    return null;
  }
  const found = transactions().find((t) => t.id === id);
  if (found === undefined) {
    return null;
  }
  return found;
}

export function addTransaction(date: string, amount: number, category: Category, note: string): boolean {
  const trimmedDate = date.trim();
  if (trimmedDate === "" || amount === 0) {
    return false;
  }
  transactions.set([...transactions(), makeTransaction(trimmedDate, amount, category, note)]);
  return true;
}

export function updateTransaction(id: number, date: string, amount: number, category: Category, note: string): void {
  transactions.set(
    transactions().map((t) => (t.id === id ? { ...t, date, amount, category, note } : t)),
  );
}

export function deleteTransaction(id: number): void {
  transactions.set(transactions().filter((t) => t.id !== id));
  if (selected() === id) {
    selected.set(null);
  }
}

export function selectTransaction(id: number): void {
  selected.set(id);
}
