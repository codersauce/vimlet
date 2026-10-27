<p align="center">
  <img src="https://raw.githubusercontent.com/codersauce/vimlet/main/assets/vimlet.png"
       alt="Vimlet: an outlet with terminal-shaped openings"
       width="144">
</p>

# Vimlet

Vimlet adds Vim editing to Rust applications. Extracted from the Red editor, it interprets keys, manages modes, registers and repeat, and returns operations for your application to apply. Your application keeps its text storage, undo history and UI.

Use a plain `String`, implement `TextSource` for your buffer, or enable the optional `ropey` adapter. Vimlet has no dependency on Red or an async runtime. Rust 1.90 or newer is required.

## Try modal editing

From the Vimlet checkout:

```sh
cargo run --locked --example quickstart
```

The example executes `cwhello<Esc>w.` on `one two three` and prints:

```text
hello hello three
```

The [complete program](examples/quickstart.rs) includes the input loop, string mutation and host acknowledgements. It finishes in Normal mode and runs as a doctest. It supports basic edits and cursor/selection changes; the guides explain how to add history and application services.

## Add Vimlet to your application

Add Vimlet to your application's `Cargo.toml`:

```toml
[dependencies]
vimlet = "0.1"
```

For an application that already uses Ropey:

```toml
[dependencies]
vimlet = { version = "0.1", features = ["ropey"] }
ropey = "1.6.1"
```

The `ropey` feature supplies text access through `TextSource`. Your application still applies the edits. Applications using other storage do not need Ropey.

## Read the guides

Build and open the documentation from the Vimlet checkout:

```sh
cargo doc --locked --no-deps --open
```

The landing page links to three guides inside rustdoc:

- **Quickstart:** dependency setup, a complete editing program, and a walkthrough of `cwhello<Esc>w.`.
- **Host integration:** event dispatch, byte coordinates, revision checks, undo grouping, services and troubleshooting.
- **Recipes:** custom mappings, macros, search repeat, composer drafts, storage and display geometry.

Keep one `EditingState` per editing surface and a `Session` for surfaces sharing registers and repeat history. Call `Session::handle` with input and a current `DocumentView`. Apply or reject each host operation, then call `Session::resume` and process its returned step too. All cursor and edit coordinates use UTF-8 bytes.

## Resolve a text object without modal editing

`MotionResolver` also works independently. This complete program finds the word around the space following `hello`:

```rust
use vimlet::{ByteOffset, MotionResolver, TextObjectKind, TextObjectScope};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let text = "hello world";
    let resolver = MotionResolver::new(&text, ByteOffset(5));
    let object = resolver
        .text_object(TextObjectScope::Around, TextObjectKind::Word, 1)
        .ok_or("no word at the cursor")?;
    assert_eq!(&text[object.ranges[0].as_range()], " world");
    Ok(())
}
```

## More examples and checks

```sh
cargo run --example string
cargo run --features ropey --example rope
cargo test --locked
cargo test --locked --features ropey
python3 scripts/conformance.py
python3 scripts/check_package.py
```

The [string host](examples/support/mod.rs) demonstrates undo grouping and macro dispatch. The [Ropey host](examples/support/rope.rs) converts byte ranges for rope mutation.

Neovim decides compatibility questions for implemented commands. The reference runner uses Neovim 0.12.1 and Python 3 to compare text, byte cursor, mode and the unnamed register. Vimlet implements modal editing; application services such as search UI, formatting and clipboard transport remain in the host. See the [scope and host contracts](docs/EXTRACTION_PLAN.md).
