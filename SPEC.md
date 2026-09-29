# Spec

Work top to bottom. Do the first unchecked item, make `cargo test` and clippy pass, check the box, commit.
"Done" for an item means its stated check passes, not that it looks right.

## Phase 1: Headless core
- [x] Document open/save/word count (`src/document.rs`)
- [x] Config load/save with defaults (`src/config.rs`)
- [x] Directory listing that respects .gitignore (`src/files.rs`)
- [x] Fuzzy filter (`src/finder.rs`)
- [ ] Text buffer wrapping cosmic-text `Buffer` with insert, delete, and cursor movement; tested without a window
- [ ] Undo/redo over the buffer (cosmic-text `vi` feature / cosmic_undo_2); tests cover undo after insert, delete, and multi-step redo
- [ ] Find in document using `nucleo`, returning match ranges; tests cover no-match and multiple matches
- [ ] Save/load a whole project folder (chapter files in order) with round-trip tests

## Phase 2: Window and rendering
- [x] wgpu surface + clear color in the winit window; `cargo build` succeeds
- [ ] Draw the buffer's text through glyphon
- [ ] Keyboard input routed into the buffer
- [ ] Mouse caret placement and drag selection

## Phase 3: App shell
- [ ] taffy layout: sidebar, editor pane, status bar with word count
- [ ] File tree in sidebar from `files::list_files`
- [ ] Command palette using `finder::fuzzy_filter`
- [ ] Open/save through rfd dialogs
- [ ] Clipboard copy/cut/paste via arboard
- [ ] Reload on external change via notify

Phase 2 and 3 items can't be checked by tests alone; the loop should stop after Phase 1 and hand back for a human to review the window.
