# Plinth

A framework for cross-platform apps: Plinth TS → Wasm component → a native host that renders a semantic UI on `gpui-ce`. See [SPEC.md](SPEC.md).

## Status: M0 (protocol and UI runtime spike)

| Part | Where |
|---|---|
| WIT world and UI API id table | `wit/plinth/` |
| Op and event encoding (ids generated from `ui-api.toml`) | `crates/plinth-protocol` |
| Semantic tree, op applier, adaptive shell, 10 controls | `crates/plinth-ui` |
| wasmtime runner (import check, time limit, memory cap) | `crates/plinth-runner-wasmtime` |
| Desktop host (`plinth-host`) | `crates/plinth-host-desktop` |
| Hand-written guest SDK (replaced by the compiler in M1) | `crates/plinth-guest` |
| CLI (`plinth example`, `componentize`, `validate`) | `crates/plinth-cli` |
| Example guests | `examples/counter-rs`, `examples/todo-rs` |

`gpui-ce` must be checked out next to this repo (`../gpui-ce`).

## Run

```sh
rustup target add wasm32-unknown-unknown
cargo run -p plinth-cli -- example todo-rs --run      # or counter-rs
```

`plinth example` builds the guest, wraps it as a component in `target/plinth/<name>.wasm`, and validates its imports. Run a built artifact directly with `cargo run --release -p plinth-host-desktop -- target/plinth/todo-rs.wasm`. Set `RUST_LOG=debug` to see the event round-trip time.

## Test

```sh
cargo test                                            # protocol and tree unit tests
cargo run -p plinth-cli -- example counter-rs
cargo run -p plinth-cli -- example todo-rs
cargo test --release -p plinth-host-desktop           # guests in wasmtime, end to end
```
