// Shared types. A separate module so `model.ts` and `storage.ts` can both
// import `Transaction` without an import cycle.
export type Category = "Groceries" | "Rent" | "Transport" | "Fun" | "Salary" | "Other";

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
