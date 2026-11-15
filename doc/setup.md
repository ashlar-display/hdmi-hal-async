# Development Setup

**Requirements:** Rust 1.85+ (stable). Install via [rustup](https://rustup.rs/).

## Clone and build

```sh
git clone https://github.com/ashlar-display/hdmi-hal-async.git
cd hdmi-hal-async
cargo build
```

## Running checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo rustdoc --all-features -- -D warnings
```

## Running tests

```sh
cargo test
```

The async tests run on `pollster`, so no runtime is needed; the README's examples run as
doctests with the others. See [testing.md](testing.md) for what the tests cover.

## `no_std` builds

The crate is `no_std` without `alloc`. CI checks it on a bare target, which has no `std`
for a dependency to pull in:

```sh
rustup target add thumbv7em-none-eabi
cargo build --target thumbv7em-none-eabi
```

## Checking the MSRV

The MSRV is `rust-version` in `Cargo.toml`. CI checks it with
[`cargo-msrv`](https://github.com/foresterre/cargo-msrv), which runs the checks on that
toolchain:

```sh
cargo install cargo-msrv --version 0.19.6 --locked
cargo msrv verify -- cargo test --locked
cargo msrv verify --target thumbv7em-none-eabi \
  -- cargo build --locked --no-default-features --target thumbv7em-none-eabi
```

## Measuring coverage

Coverage requires [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov):

```sh
cargo install cargo-llvm-cov
cargo llvm-cov
```

The current baseline is stored in `.coverage-baseline`. CI fails if coverage drops more
than 0.1 percentage points below it. On pushes to `main` or `develop`, an improvement automatically opens
a `ci/coverage-ratchet` PR to commit the new baseline.

Rust versions count lines differently, so the same code can measure differently on two
versions. Measure the baseline with the Rust version CI's `stable` uses
(`cargo +<version> llvm-cov …`), or CI can fail on a baseline that a different local
version produced.

