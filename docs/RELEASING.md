# Releasing Vimlet

Release-plz maintains a release PR with the next version and `CHANGELOG.md`.
Merging that PR publishes the crate, creates its `vX.Y.Z` tag, and creates a
GitHub Release using the same changelog entry.

## Prepare a release

1. Squash-merge changes with descriptive Conventional Commit titles:
   `feat: ...`, `fix: ...`, `perf: ...`, or `docs: ...`. Use `!` or a
   `BREAKING CHANGE:` footer for incompatible changes.
2. Review the release PR's version and changelog. Put migration instructions
   under Breaking changes and describe user-visible behavior. The SemVer check
   helps detect API breaks; it cannot detect every behavioral incompatibility.
3. Record Red and Codex qualification of the release's code revision in the PR,
   as required by the extraction plan. Requalify if runtime code changes.
4. Wait for CI, then merge the release PR. Its branch must retain the
   `release-plz-` prefix. Ordinary PR merges do not publish.
5. Check the Release workflow, the new crates.io version, the matching GitHub
   tag/release, and the docs.rs build. Documentation can finish after publication.

For a manual version adjustment on the release branch, run
`release-plz set-version vimlet@0.2.0` (substitute the intended version), then
review the manifest, lockfile, and changelog before committing.

## Validation and retries

The Release workflow reuses CI on the exact main-branch commit before publishing:
Rust 1.90 tests, Clippy, formatting, warning-free rustdoc, Neovim 0.12.1
conformance, and packaged consumers. Rust checks cover both default features and
Ropey. Publishing retains Cargo's package verification.

The release PR uses GitHub's built-in token. Because bot-created PRs do not
trigger normal PR workflows, the workflow explicitly dispatches CI on the release
branch. No personal GitHub token or GitHub App is required.

On failure, inspect the failed job before retrying. Re-run the failed workflow
on its original commit; do not move release tags. Publishing to crates.io and
GitHub is not atomic. If the crate already exists, verify its source revision
before repairing a missing GitHub Release. A new workflow dispatch runs on the
selected branch's current head, which may no longer be the failed release commit.

## One-time setup

Allow GitHub Actions to create pull requests in the repository's Actions settings.
In the crate's crates.io settings, add a GitHub Trusted Publisher:

- Repository owner: `codersauce`
- Repository: `vimlet`
- Workflow filename: `release.yml`
- Environment: leave blank

Release-plz obtains a short-lived publishing token through GitHub OIDC. Do not
add a `CARGO_REGISTRY_TOKEN` secret to this workflow. A crate must already exist
on crates.io before Trusted Publishing can be configured.

## Changelog style

`release-plz.toml` defines the section order: Breaking changes, Added, Fixed,
Performance, Documentation, then Changed for older nonconventional commits.
Empty sections are omitted. Routine chore, CI, build, test, style, and refactor
commits are filtered; breaking changes remain visible. Scopes are omitted from
bullets, PR references are linked, and each release links to the full comparison.
Review generated prose before merging; automation cannot write migration guidance
or determine which implementation details matter to consumers.
