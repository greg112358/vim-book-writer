# Book Writing App

A word editor for writing books. Rust, fully custom-drawn, one codebase for macOS and Linux.

## Stack
winit (window/input/IME) · wgpu 30 (GPU) · glyphon 0.12 + cosmic-text 0.19 with `vi` (text, editing, vim, undo) ·
taffy (layout) · rfd (file dialogs) · arboard (clipboard) · muda (menus, optional) · nucleo (fuzzy search) ·
ignore (directory walk) · notify (file watching) · serde + toml (config) · accesskit (optional) · cargo-packager.

Pinned to stable lines: winit 0.30 (0.31 is beta) and notify 8 (9 is rc). Don't bump to pre-releases.

## Layout
- `src/lib.rs` – headless core. No window, no GPU. Everything testable lives here.
- `src/main.rs` – winit entrypoint. Keep thin; logic belongs in the library.

## Commands
- `cargo test` – unit tests
- `cargo clippy --all-targets -- -D warnings` – lints
- `cargo fmt --check` – formatting
- `cargo run` – open the window
- `muda` is behind the `native-menu` feature (needs `gtk3-devel` on Linux); default builds skip it

## Rules
- Put logic in the library crate with tests. The window and GPU layers can't be asserted on in a loop.
- Editing behavior (undo/redo, cursor, selection) needs tests before it needs UI.
- Never leave `cargo test` or clippy failing when you stop.
