# Changelog

Notable changes to Vimlet. Each release uses the same notes here and on GitHub.

## [0.2.0](https://github.com/codersauce/vimlet/releases/tag/v0.2.0) — 2026-10-28

Vimlet 0.2 adds host-dispatched macros and improves repeat, Visual editing, and undo grouping.

### Breaking changes

- Extend the public `Key`, `Operator`, `Action`, and `Motion` enums. Update exhaustive matches to handle `Key::Named`, `Operator::Comment`, `Action::SelectTarget`, `Action::SwapVisualHorizontal`, and `Motion::NextLineFirstNonBlank`. Numeric discriminants of some fieldless variants have changed; do not persist enum casts as stable identifiers.
- Group a complete macro invocation, including its count and nested calls, into one undo transaction. Hosts must honor `Transaction::Begin`, `Continue`, and `End` during macro playback as well as Insert/Replace sessions.

To upgrade, change the dependency to `vimlet = "0.2"`, add the new enum match arms, and verify that one undo reverses an entire counted or nested macro invocation. Ropey consumers should retain `features = ["ropey"]`.

### Added

- Dispatch recorded keys through host keymaps with bounded `MacroPlayback` and `MacroBinding` APIs.
- Preserve host-specific named keys in macros and repeat accepted host edits within Insert sessions.
- Expose an unnamed-register revision counter for clipboard synchronization, including repeated identical yanks.

### Fixed

- Stop macro playback when a vertical motion is blocked at a document boundary. ([#1](https://github.com/codersauce/vimlet/pull/1))
- Resolve rectangular edits and characterwise Visual repeat in display cells, including short lines, tabs, and wide characters.
- Match Neovim's Visual Enter and block-corner switching behavior.
- Respect empty-line and end-of-file boundaries in change operators, and improve host-dispatched repeat and Visual puts.

### Performance

- Keep character edits local on long string-backed lines.

[Full changelog](https://github.com/codersauce/vimlet/compare/v0.1.0...v0.2.0)

## [0.1.0](https://github.com/codersauce/vimlet/tree/v0.1.0) — 2026-10-26

### Added

- Initial standalone Vim editing engine with host-owned storage and undo, string and optional Ropey adapters, integration guides, and Neovim conformance checks.
