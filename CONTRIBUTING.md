# Contributing to host-identity

Thanks for considering a contribution. This document covers the
essentials; the full conventions live in the
[Developer guide](docs/developer-guide.md).

## Ground rules

- By submitting a pull request, you agree to dual-license your
  contribution under Apache-2.0 and MIT, matching the rest of the
  project (see [`LICENSE-APACHE`](LICENSE-APACHE) and
  [`LICENSE-MIT`](LICENSE-MIT)).
- All participants are expected to follow the
  [Code of Conduct](CODE_OF_CONDUCT.md).
- Security-sensitive reports must **not** go in public issues — use
  GitHub's private vulnerability reporting on this repository.

## Getting started

```bash
git clone https://github.com/dekobon/host-identity
cd host-identity
cargo build
cargo test --all-features --workspace
```

MSRV is declared in the workspace `Cargo.toml` (`rust-version`).

For CLI contributors, [ShellSpec](https://shellspec.info/) is an
optional prerequisite for running the `spec/` suite
(`cargo xtask shellspec`). See the
[Developer guide](docs/developer-guide.md#cli-specs-shellspec).

Before pushing:

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --workspace
```

## Project conventions

The [Developer guide](docs/developer-guide.md) is the canonical
reference. Start there before making a non-trivial change. It covers:

- Rust style and error handling
- Naming conventions
- Testing discipline and assertion strength
- Conventional commit format
- GitHub CLI usage
- Documentation and changelog expectations
- Adding a new identity source (platform or cloud)

Highlights:

- **Rust style**: `cargo fmt`, clippy clean with
  `--all-targets --all-features -- -D warnings`. No `unsafe_code`.
  Avoid `unwrap` / `expect` / `panic!` outside tests.
- **Commits**: Conventional Commits — `<type>(<scope>): <subject>`.
- **Tests**: add a regression test whenever you fix a bug. Assertions
  must be specific (not `is_ok()` without checking the value).
- **Docs**: `///` on public items; the crate warns on `missing_docs`.

## Workflow

1. **Open an issue first** for anything beyond a trivial fix. It's
   cheaper to discuss the design than to review a large PR against
   the grain of the project.
2. **Branch** from `main`.
3. **Commit in small, reviewable steps.** Each commit should build
   and pass tests on its own where practical.
4. **Run fmt, clippy, and tests** before pushing.
5. **Open a pull request** against `main`. Fill in the PR template.
   Link the issue with `Fixes #NNN` in the PR body.
6. **Changelog**: add an entry to
   [`crates/host-identity/CHANGELOG.md`](crates/host-identity/CHANGELOG.md)
   under `[Unreleased]` for user-visible changes (API additions,
   behaviour changes, bug fixes). Refactors, docs-only, and CI
   changes don't need a changelog entry — the commit message is
   enough.

## Code review

Criticism is welcome — point out mistakes, suggest better approaches,
cite relevant standards. Be skeptical and concise. Reviews focus on
correctness, API shape, and test coverage before style.

## Questions

Open a GitHub Discussion or a low-priority issue. We'd rather answer
a question than review a PR that went the wrong direction.
