# Shipped cores

Each file here is a runtime core exactly as it shipped (PRODUCTION C3, C10). A core file depends on the toolchain that built it, so a later build of the same source can have other bytes. Keep every shipped file here, so that an app that pins a core (`core = "pinned"`) always finds it.

| Version | File | SHA-256 | Shipped |
|---|---|---|---|
| 1.12.0 | `1.12.0.wasm` | `f0f9ca252decdec09c236b8d3c9fa22ef32ffd7b80f6d929f6ba706b64a506e8` | 2026-10-07 |

Rules:

- Never change or remove a file here.
- Add a file when a new minor or patch version ships, with its digest in this table.
- Install one with `plinth core install cores/shipped/<version>.wasm`.
