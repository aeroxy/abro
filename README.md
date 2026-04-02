# Abro

An open-source, modern terminal application built on Tauri + React. Abro brings a Warp-style block-based UI to your command line, with bring-your-own LLM support for inline AI assistance. Designed for developers who want a fast, extensible terminal with first-class agent integration — without vendor lock-in on the AI side.

## Demo

https://github.com/aeroxy/abro/raw/main/assets/demo.mp4

## Platform Target

- Supported platforms: macOS, Windows, Linux

## Workspace Layout

```
src-tauri/           # Tauri Rust Backend
  src/               # Backend logic (Terminal PTY, Agent API, Settings)
src-ui/              # React Frontend (Tailwind CSS, Lucide Icons)
crates/
  term-core/         # Terminal/session domain model
  shell-integration/ # Shell hook generation for command boundaries
  ai-gateway/        # LLM provider abstraction layer
  persistence/       # Local state read/write
```

## Current Status

- Tracking and execution plan: `migration.md` (Migrating from GPUI to Tauri)

## Development

Make sure you have `pnpm` and `cargo` installed.

```bash
# Install Node dependencies
pnpm install

# Run the app in development mode
pnpm tauri dev

# Check Rust backend
cd src-tauri && cargo check --workspace
```

## Build

```bash
pnpm tauri build
```
