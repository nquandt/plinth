// Shared types. A separate module so `model.ts` and `storage.ts` can both
// import `Transaction` without an import cycle.
// A plain `string`, not a string-literal union: the compiler disallows `as`
// casts (PL2006), so there is no way to turn a validated `string` (from a
// `Picker` signal) back into a literal union without one. See docs/GAPS.md.
export type Category = string;

export const categories: Category[] = ["Groceries", "Rent", "Transport", "Fun", "Salary", "Other"];

export interface Transaction {
  id: number;
  /** Free-form text, for example "2026-10-05". No date parsing is needed. */
  date: string;
  /** Positive for income, negative for an expense. */
  amount: number;
  category: Category;
  note: string;
}
