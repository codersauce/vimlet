# Vimlet

- Keep consecutive `use` and `pub use` declarations together and let rustfmt order them. Leave one blank line between other top-level Rust items and between methods, including trait methods. Keep documentation and attributes attached to their items.
- Start the crate root and each public module with `//!`. Document every public API item with `///`, including variants and named fields. Lead with a short summary, then explain caller-visible details such as UTF-8 byte offsets, range endpoints and revision requirements.
- Use `//` to explain surprising behavior. Neovim decides compatibility questions. Keep design history out of implementation comments and link workarounds to their issues.
- Keep `unsafe_code = "forbid"`. If that policy ever changes, every unsafe block or impl must have a site-specific `// SAFETY:` comment explaining why its invariants hold.

Before finishing, review item and method spacing, run `cargo fmt --all -- --check`, and build the docs with `RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps` both with and without `--features ropey`.
