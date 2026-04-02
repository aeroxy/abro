# Abro: GPUI to Tauri Migration Summary

This document summarizes the complete migration of the Abro terminal application from Zed's GPUI framework to a modern Tauri + React stack, transforming it into a Warp-style block-based terminal.

## 1. Architectural Shift
* **Removed:** GPUI framework, Zed dependencies, `app-gpui` crate, and native Rust UI components (`src/ui`, `src/workspace`).
* **Added:** Tauri v2 backend, React 19 frontend, Vite, TailwindCSS v4, and Lucide icons.
* **Refactored:** Moved reusable terminal, agent, and settings logic from the old `src` directory to `src-tauri/src/`.

## 2. PTY & Shell Integration
* **Library swap:** Replaced the GPUI terminal backend with `portable-pty`.
* **Block Protocol:** Heavily customized `crates/shell-integration` to emit `__ABRO_BOUNDARY__` markers for:
  * `start`: Captures the executed command.
  * `end`: Captures the exit code for Success/Error UI indicators.
  * `cwd`: Captures the Current Working Directory to update the React UI.
  * `host`: Captures the machine hostname (local vs remote).
* **SSH Wrapping:** Created a native `ssh` wrapper in the shell integration snippet that intercepts `ssh` commands and automatically bootstraps the Abro hooks onto the remote server, enabling native block-style rendering even during SSH sessions.

## 3. Frontend UI (Warp-Style)
* **Block Layout:** Moved away from a traditional monolithic `xterm.js` canvas to individual React components (`Blocks`) for each command/output pair.
* **Output Formatting:** Implemented a robust ANSI escape sequence stripper (handling both CSI and OSC sequences like macOS path broadcasting). Used `Fira Code` with `white-space: pre` and `tab-size: 8` for perfect columnar alignment of tools like `ls`.
* **Dynamic Resizing:** Added a `ResizeObserver` that calculates character columns and syncs the exact dimensions to the Rust PTY in real-time.

## 4. Ghost Text (Predictive Auto-Completion)
* **Architecture:** Outlined a comprehensive autocompletion plan (`auto_completion.md`).
* **Implementation:** Chose a highly-performant local heuristic approach (Option B) for predictive typing. 
* **Backend:** Rust intercepts input strokes to instantly scan `~/.zsh_history`, `~/.bash_history`, and the local filesystem via `fs::read_dir`.
* **Frontend:** React renders the best prediction as semi-transparent "ghost text" directly beneath the active cursor, which can be completed via `Tab` or `Right Arrow`.

## 5. Next Steps / Current Status
* The codebase compiles cleanly in both debug and `--release` modes with zero Clippy warnings.
* **Active Issue:** The React UI currently shows a "Connecting..." state because the frontend is missing the initial `spawn_pty` callback payload in some dev environments. Debugging prints and alerts have been injected to trace the Tauri event bridge connection.