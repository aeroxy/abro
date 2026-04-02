# Abro Context Snapshot

## Project Goal
Build a macOS-only, open-source Warp-like terminal on Zed GPUI with BYO LLM support.

## Current Scope and Status
- Platform: macOS only.
- Task progression:
  - `T00` completed: workspace bootstrap.
  - `T01` completed: PTY session + tab model.
  - `T02` completed: terminal rendering + input interactions + IME + selection.
  - `T03` completed: shell boundary hooks/parser + boundary metadata.
  - `T04` in progress: block-native terminal viewport with block actions.

## Implemented So Far (High-Level)
- Shell integration boundaries for zsh/bash/fish.
- Streaming boundary parsing and suppression of hook-noise output.
- GPUI app with command input, selection/copy/paste, IME support, and password-mode behavior for sensitive prompts.
- Block model in `app-gpui`:
  - command start/end driven by boundary events,
  - output capture into active block,
  - actions: collapse/expand, copy, rerun,
  - filter modes: All / Success / Failed.
- Persistence model extended in `persistence` crate:
  - block list + filter mode saved in `abro-state.json`.

## Latest User-Reported Issues (Addressed)
1. Block outputs showed `no output captured`.
   - Fixed by handling boundary start before output, and end after output in pump processing.
2. Block output text not selectable/copyable.
   - Added per-block output selection layouts, mouse selection handlers, and `cmd-c` copy behavior.
3. Block order reversed.
   - Changed visible block ordering to chronological (oldest -> newest).
4. Stale startup block (e.g., old `ls`) appeared on launch.
   - Default launch now starts fresh block stream.
   - Optional restore preserved behind env flag: `ABRO_RESTORE_BLOCKS=1`.

## Current UX Model
- Terminal viewport is block-native (Warp-like): the viewport renders command blocks directly.
- Filter chips and block actions are inline inside the viewport.
- Command input remains below viewport.
- Surface styling now uses a Warp-like visual shell:
  - warm paper background and muted blue text,
  - compact top chrome row with tab pill,
  - docked bottom composer panel,
  - softer beige block cards/output areas.
- Layout structure now also mirrors Warp more closely:
  - large empty upper canvas region,
  - lower bounded stream panel for command blocks,
  - composer docked beneath stream panel inside the lower region.
- macOS titlebar behavior now follows Zed’s pattern:
  - transparent titlebar enabled,
  - traffic lights repositioned via `traffic_light_position`,
  - top bar content inset to avoid overlap and act as drag area.
- Visual tuning pass applied for native feel:
  - top content now starts at y=0 to align with titlebar controls,
  - titlebar inset widened for control clearance,
  - corner radii reduced from medium to small across top shell/stream/composer/cards.

## Important Files
- Main app UI/runtime: `/Users/aero/Documents/abro/crates/app-gpui/src/lib.rs`
- Boundary parser/hooks: `/Users/aero/Documents/abro/crates/shell-integration/src/lib.rs`
- PTY/tab manager + boundary plumbing: `/Users/aero/Documents/abro/crates/term-core/src/lib.rs`
- Persistence schema/store: `/Users/aero/Documents/abro/crates/persistence/src/lib.rs`
- Live plan tracker: `/Users/aero/Documents/abro/plan.md`
- Manual smoke checklist: `/Users/aero/Documents/abro/docs/manual-smoke-macos.md`

## Validation State
- Recent checks passed:
  - `cargo test -p app-gpui --all-targets`
  - `cargo check --workspace --all-targets`
  - `cargo check -p app-gpui`
- Latest bundle built:
  - `/Users/aero/Documents/abro/target/aarch64-apple-darwin/debug/bundle/osx/Abro.app`
  - `/Users/aero/Documents/abro/target/aarch64-apple-darwin/release/bundle/osx/Abro.app`
  - `/Users/aero/Documents/abro/target/aarch64-apple-darwin/release/Abro-aarch64.dmg`

## Immediate Next Focus (T04 Completion)
- Run manual smoke for block UX and persistence paths.
- Add/verify E2E coverage for block actions + persisted reload acceptance gate.
- Once T04 gate is green, move to T05 (AI provider abstraction).
