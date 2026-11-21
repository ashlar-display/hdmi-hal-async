# Testing strategy

hdmi-hal-async defines the async traits and their `Send` twins; the value types they carry
come from hdmi-hal. Its tests check that the traits can be implemented and called as
documented, using small mock implementations driven by `pollster`, and that the twins'
futures are `Send` where generic code needs them to be. Behaviour against real hardware is
tested in the backends and in the crates that drive the traits.

## Test structure

### Unit tests

Unit tests live in a `#[cfg(test)]` module at the end of each file. Async tests run on
`pollster` (`#[pollster::test]`), a minimal executor, so no runtime is needed.

| File | What the tests check |
|---|---|
| `src/phy/mod.rs` | A mock `HdmiPhy` records each call: FRL rate, scrambling, FRL output, link training patterns and equalization are applied and can be changed; a `SendHdmiPhy` is usable as an `HdmiPhy`, and its futures are `Send` in generic code; the value types re-exported here are hdmi-hal's, at both paths |
| `src/scdc/mod.rs` | A mock `ScdcTransport`: writes read back, registers are independent, unwritten registers read zero; `read_block_bytewise` reads consecutive registers, reads nothing for an empty buffer, stops at the first error, wraps past `0xFF`, and works on an unsized transport and through a `SendScdcTransport`; a `SendScdcTransport` is usable as an `ScdcTransport`, and its futures are `Send` in generic code |

### Doctests

The README's Rust examples run as doctests (`ReadmeDoctests` in `src/lib.rs`). The
backend examples are sketches: their types are defined in hidden lines and their method
bodies are `todo!()`, so they are compiled against the traits but never run. The
multi-threaded example spawns through a hidden stand-in for `tokio::spawn` with the same
`Send + 'static` bound, so it checks that the `Send` twin's futures can be spawned without
depending on tokio.

## Builds

The crate has no features. CI runs the tests on stable and on the MSRV, and builds the
crate for `thumbv7em-none-eabi`, which has no `std`, on both.

## Coverage

Coverage is measured with `cargo-llvm-cov` and kept from falling by a ratchet: CI fails
when line coverage drops below `.coverage-baseline` (with a 0.1 point tolerance), and
opens a PR raising the baseline when a push improves it. See
[setup.md](setup.md#measuring-coverage) for the commands.

## Philosophy

- **Test the contract, not an implementation.** A mock records what the trait was asked
  to do; the tests check the calls and values the documentation promises.
- **`Send` is part of the contract.** The twins exist so generic code can spawn their
  futures; the tests check that in generic code, where it matters, not only for one
  concrete type.
