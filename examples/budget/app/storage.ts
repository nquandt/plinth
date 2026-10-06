// Persists transactions to the per-app key/value store (SPEC.md §8.5).
// Needs the `store.kv` capability, declared in plinth.toml.
import { kv } from "plinth:store";
import { JSON } from "plinth:core";
import type { Transaction } from "./types";

const KEY = "transactions";

/** Writes `transactions` to the store, replacing whatever was saved before. */
export function saveTransactions(transactions: Transaction[]): void {
  kv.set(KEY, JSON.stringify(transactions));
}

/** Reads the saved transactions, or `null` if nothing was saved yet (or the
 * saved text does not parse as a `Transaction[]`). */
export function loadTransactions(): Transaction[] | null {
  const raw = kv.get(KEY);
  if (raw === null) {
    return null;
  }
  return JSON.parse<Transaction[]>(raw);
}
