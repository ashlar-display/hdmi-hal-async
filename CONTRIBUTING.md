# Contributing to hdmi-hal-async

Thanks for your interest in contributing. This document covers the basics.

## Getting started

See [`doc/setup.md`](doc/setup.md) for build and check instructions.

## Issues and pull requests

**Open an issue first** if you're unsure whether something is a bug or if you want to
discuss a change before implementing it. For small, self-contained fixes a PR on its own
is fine.

- Bug reports: describe the trait method or associated type in question and what contract
  it fails to express correctly.
- Feature requests: a brief description of what you need and why is enough to start a
  conversation.
- PRs: keep them focused. One logical change per PR makes review faster and keeps
  history readable.

## Coding standards

- Run `cargo fmt` and `cargo clippy --all-targets -- -D warnings` before pushing
  ([doc/setup.md](doc/setup.md) lists the commands CI runs).
- Public items need rustdoc comments, and intra-doc links must resolve
  (`cargo rustdoc --all-features -- -D warnings` must pass).
- `#![forbid(unsafe_code)]` is enforced; no unsafe code.
- A claim about HDMI behaviour (a rule, a default, a threshold, a register encoding) cites
  at least two independent public implementations (repository, commit, file and line).
  Where implementations differ, the behaviour is a setting with the majority as default.
- All traits and public types must be usable in bare `no_std` environments. No trait
  method may require allocation.
- All public structs and enums must be `#[non_exhaustive]`.

## Commit and PR expectations

- Write commit messages in the imperative mood ("Add support for …", not "Added …").
- Keep commits logically atomic. A PR that touches three unrelated things should be
  three commits (or three PRs).
- CI must be green before a PR can merge: fmt, clippy, docs, the tests and the
  bare-metal `no_std` build, the same tests and build on the MSRV (Rust 1.85), and
  coverage must not drop more than 0.1 percentage points below the baseline (stored in
  `.coverage-baseline`).

## Trait stability

Trait changes are breaking changes for every implementor. The bar for modifying a trait
surface after initial publication is high. New methods require a default implementation
or a semver-breaking release. Additions and modifications must be reviewed against all
known consumers in the stack.

## Publishing and downstream dependencies

`hdmi-hal-async` builds on [hdmi-hal](https://github.com/ashlar-display/hdmi-hal) and
re-exports its value types, so moving to a breaking release of hdmi-hal is a breaking
release of hdmi-hal-async, unless the version requirement keeps accepting the old release
as well. It is consumed by `culvert-async`, `plumbob-async`, and async platform
backend crates as a versioned dependency. Changes that add or modify public API must be
merged, tagged, and published to crates.io before downstream crates can adopt them.
The workflow is:

1. Merge and tag a release in this repo (`vX.Y.Z`).
2. The publish CI workflow uploads the new version to crates.io.
3. Update the version constraint in the downstream crate's `Cargo.toml`.

## Review process

PRs are reviewed on a best-effort basis. Expect feedback within a few days; if you
haven't heard back in a week feel free to ping the thread. Reviews aim to be
constructive — if something needs to change, the reviewer will explain why. Approval
from the maintainer is required to merge.

## Code of Conduct

This project follows the [Contributor Covenant 3.0](https://github.com/ashlar-display/.github/blob/HEAD/CODE_OF_CONDUCT.md),
the code of conduct of every [Ashlar](https://ashlar-display.org) repository. Please read it
before participating.
