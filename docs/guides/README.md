# Guides for app authors

These guides tell you how to do one task in a Plinth app. Read [getting-started.md](../getting-started.md) first. The references are [language.md](../language.md), [ui.md](../ui.md) and [host-apis.md](../host-apis.md). [LIMITS.md](../LIMITS.md) tells what Plinth cannot do yet.

| Guide | Read it when you want to |
|---|---|
| [storage.md](storage.md) | Save data: the `kv` store and the private file space, the capabilities, the quotas, and where each host keeps the data. |
| [network.md](network.md) | Send HTTP requests with `fetch`: `net:<host>`, `net:*` and `net.local`, private addresses, redirects, the body limit and the errors. |
| [updates.md](updates.md) | Ship new versions: the `[updates]` section, the signed feed, `plinth feed add`, and the core policy (`follow`, `patch`, `pinned`). |
| [publishing.md](publishing.md) | Put your app in a registry: the publisher key, `plinth sign` and `plinth build --sign`, `plinth registry build` and `serve`, and what the Hub checks. |
| [accessibility.md](accessibility.md) | Make your app accessible: what Level 1 controls give you, what you must do with Level 2 primitives, and how to test with axe. |
| [performance.md](performance.md) | Keep your app fast: flat layouts, long lists, `onFrame`, release builds, and the time and memory limits. |

Each example in these guides was checked with `plinth check` against the typings in `std/*.d.ts`.
