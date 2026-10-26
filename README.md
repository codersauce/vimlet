# Vimlet

Vimlet is a headless Vim editing engine for Rust, extracted from the Red editor. Give it a key and a borrowed document view. It interprets the key, tracks the mode, and returns an operation for the host to apply. Red can enable the optional Ropey adapter. A string-backed application can use the default feature set.

The resolver also works independently of the interpreter.

```rust
use vimlet::{ByteOffset, MotionResolver, TextObjectKind, TextObjectScope};

# fn example() -> Option<()> {
let text = "hello world";
let resolver = MotionResolver::new(&text, ByteOffset(5));
let object = resolver.text_object(TextObjectScope::Around, TextObjectKind::Word, 1)?;
assert_eq!(&text[object.ranges[0].as_range()], " world");
# Some(())
# }
# assert!(example().is_some());
```

## Integrating an editor

Keep one `EditingState` per editing surface and one `Session` per shared register and repeat scope. Supply `DocumentView` with UTF-8 byte coordinates, a document identity and revision, the cursor, and any Visual selection. An opaque span makes an attachment or placeholder a single motion unit. A host can supply display geometry for tab stops and wide characters.

1. Ask the state for its binding context and resolve the original key through the host's keymap. Call `Session::handle` with both the original key and its binding.
2. For `Step::HostOperation`, check `operation.is_current(version)` immediately before mutation. Apply replacements against the original revision in descending byte order. Use the transaction phase to group undo and report the actual changes and cursor through `Session::resume`.
3. For `Step::Replay`, send the key through the host's ordinary dispatch. After the host finishes the key, request the next one with `next_macro_input`. Outstanding operations suspend playback. End the editing context with `lifecycle` when a draft is submitted or replaced.
4. Use `Step::Unhandled` for application shortcuts. Search prompts, clipboard transport, syntax, undo storage and display layout belong to the host. A search can return a remembered target so dot resolves the saved query at the new cursor.

The string example implements the full dispatch loop and a small undo history. The rope example applies byte ranges directly to a rope. [String host](examples/support/mod.rs) · [Rope host](examples/support/rope.rs)

## Run locally

```sh
cargo run --example string
cargo run --features ropey --example rope
cargo test --locked
cargo test --locked --features ropey
python3 scripts/conformance.py
python3 scripts/check_package.py
```

Rust 1.90 is the minimum toolchain. The reference runner uses Neovim 0.12.1 and Python 3. It compares actual mapped input, including incomplete commands, and observes text, byte cursor, mode and the unnamed register. The published library only uses Rust dependencies. [Scope and host contracts](docs/EXTRACTION_PLAN.md)
