# Start a new agent session

Copy the prompt below into a new session in `C:\repos\plinth`. The last session ended on 2026-10-29. `master` is pushed and the full gate passed.

## The prompt

> You continue work on Plinth (Plinth TS to `.plnt` apps; gpui desktop host; DOM web host). Read `HANDOFF.md` first (sections 0, 0.1, 0.2, 7, 8 and 10), then `docs/HISTORY.md` (the sections from 2026-10-27 to 2026-10-29) and `docs/KANBAN-FINDINGS.md`.
>
> Rules from the owner: orchestrate Sonnet agents in isolated worktrees (each with its own `CARGO_TARGET_DIR`); one bounded step for each agent; merge, run `bash scripts/ci-local.sh full` (set `PLAYWRIGHT_PATH=C:/repos/orchestra/node_modules/playwright`), read the log for "ci-local: all checks passed (full)", then push; remove the merged worktree and its target folder. Commit after each tested step. No GPG signing. No attribution lines. Write docs in ASD-STE100 Simplified Technical English. Keep the token approach for color and style (no raw colors or pixels). Do not mark Linux or macOS items Done from Windows.
>
> State: Windows is the only tested platform. The registry `https://plinth.femur.dev/` is live behind Cloudflare Access. UI API 1.14, `plinth:secrets`, the app `[theme]`, `examples/kanban` and the combined data quota are built.
>
> Next, in this order (pick the first that the owner has not changed):
> 1. A keyed `List` for cards, so guest memory and frame cost stop growing with the card count (`docs/KANBAN-FINDINGS.md` H26). Use `examples/kanban` as the test.
> 2. `Chart` height (H6) and roving focus (H21).
> 3. The test kit: drop, key and context-menu steps (H22 to H24).
> 4. A `warning` token and the compiler friction items (H11 to H15).
> 5. Anything in `docs/PRODUCTION.md` that Windows can finish (`bash scripts/production-status.sh --open Beta`).
>
> The owner does the Mac and iPhone work. Do not wait for it. Ask the owner only for a decision that no document answers.

## State at the end of the last session

- Pushed to `origin/master`; the working tree was clean.
- No agent runs. Two empty locked folders under `.claude/worktrees/` remain (Windows locks them; a restart frees them).
- Open for the owner: `docs/HANDOFF.md` section 0.2.
