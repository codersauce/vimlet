# Standalone extraction

Milestones 1–4 establish the independent crate that Red and Codex will consume. The next integration is Red's embedded textarea and main editor. Codex follows once the Red integration passes its tests. Both consumers must qualify the same revision before release.

| Milestone | Delivered surface | Verification |
| --- | --- | --- |
| 1. Foundation | Independent Cargo package, Red source provenance, example hosts and pinned Neovim runner | Compare the trace driver with isolated Neovim using real typed input |
| 2. Text and motions | Borrowed string and optional rope access, grapheme traversal across chunks, logical lines, display columns, opaque elements, operator ranges and text objects | Run shared adapter and Unicode fixtures. Inspect each external consumer's enabled dependencies |
| 3. Interpreter | Normal, Insert, Replace and three Visual modes, configurable mode capabilities, counts and literal prefixes, checked host operations and lifecycle transitions | Exercise incomplete commands, cancellation, suspended operations, stale revisions and host-adjusted edits |
| 4. Repeat and macros | Counted dot, Insert and Replace replay, Visual edits, shared text and macro registers, bounded recording and playback with host dispatch | Compare Neovim traces and exercise keymap changes, failed targets, register updates and draft replacement |

## Command coverage

The resolver handles logical `h j k l`, line starts and ends, word and WORD starts and ends in both directions, line and percentage jumps, single-character find and till with repeat, delimiter matching, paragraphs and sentences. Text objects cover words, WORDs, paragraphs, sentences, balanced parentheses, brackets and braces, and quoted strings.

The interpreter composes motions with delete, change, yank and case operators. It supports line forms, insert and append, opening lines, single-character and continuous replacement, puts, joins, Visual selection and block editing. Explicit register inspection and replacement are available through the session. Macro names accept letters and digits, uppercase recording appends, and automatic yank and numbered delete updates use the same register bank.

Neovim decides shared behavior. An accepted empty Insert session can become the last change. A failed change target cancels the insertion tail of dot. The conformance corpus includes both behaviors. [Reference cases](../tests/conformance.json)

## Host contracts

A host owns its storage, undo grouping, cursor and anchors, application dispatch and search dialect. It supplies results for search, marks, syntax, block selection and formatting. It must check every resolved edit against its document identity and revision immediately before applying it. A host can report rejected operations and return actual inserted and removed text for accepted edits. The engine uses this feedback for repeat and register state.

`Transaction::BreakInsert` closes an undo group when the cursor moves during insertion. A later edit opens a new group. A host switching documents during an active transaction must close its history group as part of that transition.

Shared editing has a finite command inventory. Ex, file and window commands, syntax services, regex matching, application UI and multi-cursor workflows belong to consumer integrations. The conformance suite covers the recorded cases. Real consumer validation belongs to milestones 5 and 6.
