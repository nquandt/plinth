// The SQLite Worker of a page that shows Plinth apps (docs/SQL.md §2): a
// module Worker of the page's own origin, so it has OPFS. `sql.js` has the
// engine and the message rules; `SqlWorkerHost` (in the page) starts this
// Worker at the first `store.sql` call of an app.

import init from "./vendor/sqlite-wasm/sqlite3.mjs";
import { serveSql, startSqlEngine } from "./sql.js";

serveSql(self, () => startSqlEngine(init));
