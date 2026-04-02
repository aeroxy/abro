# Project Structure Summary

## Overview

Abro is a terminal application with agent integration built using Tauri, React, and TypeScript. It features a modern, Warp-style block-based UI.

## Key Files & Directories

- `src-tauri/` - Rust backend application (Tauri)
  - `src-tauri/src/main.rs` - Application entry point
  - `src-tauri/src/terminal/` - Terminal and PTY integration components
  - `src-tauri/src/agent/` - Agent server integration
  - `src-tauri/src/settings/` - Configuration management
- `src-ui/` - React frontend user interface components
  - `src-ui/App.tsx` - Main application React component
  - `src-ui/index.css` - TailwindCSS styling

## Build Instructions

Using `pnpm` is highly recommended for managing Node dependencies and Tauri commands.

```bash
# Install dependencies
pnpm install

# Run application in development mode
pnpm run dev
# or
pnpm tauri dev

# Build for production
pnpm run build
# or
pnpm tauri build
```
