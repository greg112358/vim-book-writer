# Vim Book Writer

A fast, lightweight word processor for writing in djot (`.dj`), with vim controls. Rust, fully custom-drawn, one codebase
for macOS and Linux. Read `SPEC.md` for the product and the task list.

## Stack
winit (window/input/IME) · wgpu 30 (GPU) · glyphon 0.12 (text; it brings cosmic-text for shaping and fonts) ·
rfd (file dialogs) · arboard (clipboard) · jotdown (djot parser) · printpdf (PDF export) · ignore (directory walk) ·
notify (file watching) · serde + toml (config) · png (icons, screenshots) · cargo-packager.
The vim engine, undo, layout of the window's regions, and the djot styling are all in this repo.

Pinned to stable lines: winit 0.30 (0.31 is beta) and notify 8 (9 is rc). Don't bump to pre-releases.

## Layout
- `src/lib.rs` – headless core. No window, no GPU. Everything testable lives here.
- `src/main.rs` – winit entrypoint. Keep thin; logic belongs in the library.

## Commands
- `cargo test` – unit tests
- `cargo clippy --all-targets -- -D warnings` – lints
- `cargo fmt --check` – formatting
- `cargo run` – open the window
- `cargo packager --release` – build installers for this platform into `target/packages` (deb on Linux, app and dmg on macOS); install the tool with `cargo install cargo-packager --locked`. Settings live in `[package.metadata.packager]` in `Cargo.toml`
- `cargo run -- --file book.dj --screenshot out.png --mode insert` – draw one frame to a PNG to check a visual change

## Code style
- Prefer large functions over splitting simple logic into many small calls.
- Deduplicate only after the third repetition.
- Use as few abstractions as get the job done. No traits, wrappers, or generics without a present need.
- Write tests sparingly. Skip tautological and trivial ones. Test real logic with real edge cases (parsing, editing,
  vim behavior, export). Keep changes frictionless: a test that breaks on every refactor is a bad test.
- A test must check behavior that could plausibly break, not restate the implementation. No tests that copy a
  lookup table, a list of labels, layout constants, colors, or exercise only a dependency (serde, std, fs). They
  add friction with nearly zero value, even when an LLM is the one maintaining them.
- Keep it fast: no needless allocation or cloning on the typing and render paths.

## Rules
- Put logic in the library crate so it stays testable. The window and GPU layers can't be asserted on in a loop.
- Never leave `cargo test` or clippy failing when you stop.
