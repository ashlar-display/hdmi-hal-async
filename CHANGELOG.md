# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Breaking changes

- **`read_block` is a required method** of `ScdcTransport` and `SendScdcTransport`, as in
  hdmi-hal. The default implementations (one `read` per byte) are gone: a transport whose
  author never considered block reads would read related registers, such as the CED
  counters, one at a time and could return values torn by a sink update, with nothing to
  show for it. A transport that reads several registers in one DDC transaction implements
  `read_block` with it; one that cannot adds `use hdmi_hal_async::scdc::read_block_bytewise;`
  and `async fn read_block(&self, reg: u8, buf: &mut [u8]) -> Result<(), Self::Error> {
  read_block_bytewise(self, reg, buf).await }`, which behaves as the old default did, for
  either trait.
- **`read_block`'s contract is written down, and it is stricter than the old default**,
  mirroring hdmi-hal. An existing implementation must be checked against it: the whole
  buffer as one combined transfer to the layer below, or an error — never split, never a
  fallback to one register at a time (a bus driver known to split it is named in the
  transport's docs); calls polled together from one task (`join!`) are serialized, never
  a panic, or the second one is a documented error; no consistent snapshot promised (that
  is the sink's); a maximum length, if any, documented and of at least 16 bytes, above
  which a read is an error; an empty buffer is `Ok` with no bus activity; on error the
  buffer is unspecified; a read past `0xFF` is unspecified but never panics. The old
  default's wrap to `0x00` past `0xFF` is no longer promised (`read_block_bytewise` still
  wraps).
- **Requires hdmi-hal 0.6.** The PHY types re-exported from hdmi-hal (in `phy` and at the
  crate root: `EqParams`, `FrlOutput`, `LaneEqParams`, `LanePatterns`, `LtpPattern`,
  `TxFfeLevel`, `TxFfeRestrictions`) are now hdmi-hal 0.6's. Code that names them through
  both this crate and hdmi-hal 0.5 sees two different types; move hdmi-hal to 0.6
  alongside this crate. `HdmiForumFrl` is display-types' and is unaffected.

### Added

- **`scdc::read_block_bytewise(transport, reg, buf)`** — `read_block` as one `read` per
  byte, for transports that cannot burst-read; the documented exception to the "one
  transaction" rule. Generic over `T: ScdcTransport + ?Sized`; a `SendScdcTransport` uses
  it too, and its future stays `Send`.
- **"Errors" on `ScdcTransport`:** after an error an implementation should stay usable for
  the next call; one that cannot documents how to recover.
- **Cancellation** in `ScdcTransport`'s documentation: a dropped read returns nothing, after
  a dropped `read_block` the buffer is unspecified, after a dropped write it may or may
  not have reached the sink; an
  implementation should leave the transport usable, and one that cannot documents what a
  cancelled call leaves behind and how to recover. To stop early, check a deadline before
  each call rather than dropping one in flight. `doc/architecture.md` records why it is
  "should" (a survey of async I²C drivers' behaviour on a dropped transfer).

### Changed

- **Part of Ashlar** — the README now opens with a block naming the
  [Ashlar](https://ashlar-display.org) stack and linking the other crates in it. Keywords
  are aligned across the stack, with `ashlar` and `hdmi` on every crate; `ashlar` is added,
  `embedded` is dropped in favour of more specific keywords. Categories gain
  `no-std::no-alloc`. The crate's `homepage` is now the Ashlar website.

- **The `Send` twins' documentation says to bound generic code by the twin alone.** An
  added `+ HdmiPhy` or `+ ScdcTransport` bound, which the twins imply anyway, makes the
  compiler use it instead of the blanket implementation, so the futures are no longer
  known to be `Send`. `SendHdmiPhy`, `SendScdcTransport`, the README and architecture.md
  say so.

## [0.2.1] - 2026-10-17

### Added

- **`TxFfeRestrictions` re-exported** from `hdmi-hal`, in `hdmi_hal_async::phy` and at
  the crate root like the other PHY types. `EqParams` carries it since hdmi-hal 0.5.1
  (`tx_ffe_restrictions`), so a PHY implementation can name the type without a direct
  `hdmi-hal` dependency. Requires hdmi-hal 0.5.1.
- **`phy::SendHdmiPhy`, `HdmiPhy` with `Send` futures** — the same methods and contract,
  each returning a future that is `Send`, with `type Error: Send`. Code generic over a
  PHY can then spawn link training on a multi-threaded executor (Tokio's multi-thread
  runtime), which `HdmiPhy`'s `async fn`s do not allow. Every `SendHdmiPhy` is an
  `HdmiPhy` through a blanket implementation, so it also works with code written against
  `HdmiPhy`; implementations still write `async fn`. A type implements one of the two;
  import the one the code is bounded by (with both in scope, method calls on a
  `SendHdmiPhy` type are ambiguous).
- **`scdc::SendScdcTransport`, `ScdcTransport` with `Send` futures** — the same for the
  SCDC transport, with `type Error: Send` and its own default `read_block` (one `read`
  per byte, a `Send` future). The trait requires `Send + Sync`: `read` takes `&self`, and
  a future holding `&self` is only `Send` when the transport is `Sync`. The blanket
  `ScdcTransport` implementation forwards `read_block` too, so a transport's block-read
  override is used by code written against either trait.

## [0.2.0] - 2026-10-11

### Breaking changes

- **`LtpPattern` is now a `#[non_exhaustive]` enum with the HDMI 2.1 pattern values**,
  following `hdmi-hal`. The re-exported type previously documented 1–4 as LFSR 0–3, which
  was wrong: the values are 1 = all ones, 2 = all zeros, 3 = Nyquist clock, 4 = DDE
  compliance and 5–8 = LFSR 0–3. `LtpPattern::new(u8)` is removed; use the variants
  (`LtpPattern::Lfsr0` etc.).
- **`HdmiPhy::send_ltp` takes the full per-lane pattern set**, mirroring `hdmi-hal`: the
  signature is now `async fn send_ltp(&mut self, patterns: LanePatterns)`. `None` on a
  lane means no training pattern; `None` on every lane stops the training patterns.
- **`HdmiPhy::set_frl_output` is a new required method**, mirroring `hdmi-hal`: existing
  implementations must add
  `async fn set_frl_output(&mut self, output: FrlOutput) -> Result<(), Self::Error>`. It
  selects gap-only output during link training and active output once training has
  succeeded.
- **`ScdcTransport::read` now takes `&self` instead of `&mut self`.** Mirrors the
  same change in `hdmi-hal`. Implementations must change `async fn read(&mut self, reg: u8)`
  to `async fn read(&self, reg: u8)` and introduce interior mutability for any state
  mutated during reads.
- **`hdmi-hal` updated to 0.5** — the release with the per-lane `send_ltp`,
  `set_frl_output` and the HDMI 2.1 `LtpPattern` values mirrored above. It uses
  display-types 0.4, like this crate.
- **`display-types` updated to 0.4** — tracks DisplayID 2.x support added in `piaf` 0.4.1.

### Added

- The PHY value types (`EqParams`, `FrlOutput`, `LaneEqParams`, `LanePatterns`,
  `LtpPattern`, `TxFfeLevel`) re-exported in `hdmi_hal_async::phy` as well, at the same
  paths as in `hdmi-hal`'s `phy` module, so code moving between the sync and async traits
  keeps its imports. The crate-root re-exports remain.
- `HdmiForumFrl` re-exported at the crate root, from `hdmi-hal`.
- `LanePatterns` re-exported from `hdmi-hal` at the crate root.
- `FrlOutput` re-exported from `hdmi-hal` at the crate root.
- `TxFfeLevel` re-exported from `hdmi-hal` at the crate root. `LaneEqParams` (from
  `hdmi-hal`) now carries the lane's TxFFE level as `tx_ffe_level`, applied through
  `HdmiPhy::adjust_equalization`.
- `ScdcTransport::read_block(reg, buf)`, mirroring `hdmi-hal` — reads consecutive
  registers into `buf`. The default implementation calls `read` once per byte, so existing
  transports need no changes; transports that can burst-read should override it.
- **SLSA Build Level 2 provenance** — release artifacts are attested via
  `actions/attest-build-provenance` and verified with
  `gh attestation verify <file> --repo DracoWhitefire/hdmi-hal-async`.

### Changed

- `HdmiPhy`'s documentation now describes what it covers, matching `hdmi-hal`: what FRL
  link training needs from the transmitter (rate, training patterns, output mode,
  scrambling and each lane's TxFFE level). Further PHY configuration (lane mapping, TMDS
  pre-emphasis and swing, spread-spectrum clocking, power states) is listed as planned in
  the roadmap.
- `HdmiPhy::set_frl_rate`'s documentation says it returns once the PHY transmits at the
  rate, including any bring-up its hardware needs (such as a clock pattern held until its
  PLL locks).

### Internal

- **CI builds for a `no_std` target** — the `Build (no_std)` step now builds for
  `thumbv7em-none-eabi`. It previously built for the host, where `std` is always
  available, so it could not catch a dependency that enables `std` (as hdmi-hal did
  through `display-types`).
  The publish workflow runs the same build steps.
- **Publishing uses crates.io trusted publishing** — `publish.yml` obtains a short-lived
  token through OIDC (`rust-lang/crates-io-auth-action`, revoked at the end of the job, and
  short-lived regardless) instead of the long-lived `CARGO_REGISTRY_TOKEN` secret. The
  upload job runs in the `release` environment, which only `v*` tags can deploy to and
  which requires a maintainer's approval. Quality gates moved to a separate `verify` job,
  so no dependency build scripts run and no cache is restored where publishing credentials
  exist. The attested `.crate` is kept as a separate copy; after upload the job fails if
  crates.io's checksum differs from it, and that same copy is attached to the GitHub
  release. Re-running a partially failed publish reuses an existing GitHub release.
  `publish.yml` refuses tags that do not match the `Cargo.toml` version.
- **Release summary before approval** — `publish.yml` gains a `summary` job that shows the
  crate, version and commit, changes since the previous published release (ignoring
  pre-release or malformed tags, and flagging a baseline that is not on crates.io), changes
  to workflow, toolchain and build configuration, manifest changes, the changelog section,
  and the `.crate` sha256 with a command to reproduce it, on the run page while the publish
  job waits for approval. The publish job refuses to attest a package whose sha256 differs
  from the reviewed one.
- **Release prep and tagging workflows** — releases no longer need a manual promotion.
  `release-prep` picks the version bump with `cargo semver-checks` (or validates a chosen
  one), stamps the changelog and opens a `Release vX.Y.Z` PR into `develop`. `release-tag`
  then promotes `develop` to `main` through a `release-promote/v*` PR, tags only when `main`
  holds exactly the promoted tree, and dispatches `publish.yml` for the tag (which
  `workflow_dispatch` now allows; dispatches against a non-tag ref are skipped). Both fail
  when started from a branch other than `develop`.
- **No CI run on promotion PRs** — CI no longer runs on pull requests into `main`. The only
  ones are `release-tag`'s `release-promote/*` PRs, whose content already passed CI on
  `develop`; opened by `github-actions[bot]`, their runs would be held for approval and
  listed as `action_required` or failed.
- **Supply-chain hardening for CI** — every third-party action is pinned to a commit SHA
  of a release tag, with `dependabot.yml` keeping the pins (and `Cargo.lock`) current. The
  Rust toolchain itself still follows `stable`. `cargo-semver-checks` is installed as a
  checksum-verified prebuilt binary (the version listed by the pinned `install-action`).
  The security audit also runs weekly.
- **README documents release provenance** — a badge shows whether crates.io requires trusted
  publishing for this crate, and "Verifying releases" now states which versions carry SLSA
  provenance and a GitHub release (from 0.2.0, not every release as it previously claimed),
  that releases from 0.2.0 are uploaded through trusted publishing while 0.0.1 and 0.1.0
  were uploaded with an API token, and how to look up a version's publishing details on
  crates.io.

## [0.1.0] - 2026-04-03

### Added

- `scdc::ScdcTransport` — async mirror of `hdmi_hal::scdc::ScdcTransport`. Methods:
  `async fn read(reg: u8) -> Result<u8, Self::Error>` and
  `async fn write(reg: u8, value: u8) -> Result<(), Self::Error>`.
- `phy::HdmiPhy` — async mirror of `hdmi_hal::phy::HdmiPhy`. Methods:
  `async fn set_frl_rate`, `async fn send_ltp`, `async fn adjust_equalization`,
  `async fn set_scrambling`.
- Re-exports of `LtpPattern`, `EqParams`, and `LaneEqParams` at the crate root,
  mirroring the position of those types in `hdmi-hal`.

**Project infrastructure**

- `#![no_std]`, `#![forbid(unsafe_code)]`
- `#![allow(async_fn_in_trait)]` — targets single-threaded embedded executors (e.g.
  Embassy) where `Send` bounds on futures are not required, following the same
  decision made by `embedded-hal-async`.
- Full rustdoc coverage enforced via `cargo rustdoc -- -D missing_docs`.
- Unit tests with mock implementors in each trait module, using `pollster` as the
  test executor.

## [0.0.1] - 2026-04-03

- Name claimed in advance of implementation. No traits are implemented yet.
  See [`doc/roadmap.md`](doc/roadmap.md) for the planned 0.1.0 scope.
