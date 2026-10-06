// Shared types. A separate module so `model.ts` and `storage.ts` can both
// import `Note` without an import cycle.
export interface Note {
  id: number;
  title: string;
  body: string;
}
