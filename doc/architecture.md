# hdmi-hal-async — Architecture and Implementation Plan

## Overview

`hdmi-hal-async` is the async companion to `hdmi-hal`, following the same design split
as `embedded-hal` / `embedded-hal-async`. It defines async versions of the two traits
in `hdmi-hal`:

- `scdc::ScdcTransport` — raw byte-level SCDC register access
- `phy::HdmiPhy` — what FRL link training needs from the transmitter PHY: FRL rate, link
  training patterns, FRL output mode, scrambling and each lane's TxFFE level

and a `Send` twin of each, `scdc::SendScdcTransport` and `phy::SendHdmiPhy`, for code that
spawns on a multi-threaded executor (see "`Send` futures: the twins" below).

Data types (`LtpPattern`, `LanePatterns`, `FrlOutput`, `EqParams`, `LaneEqParams`,
`TxFfeLevel`, `TxFfeRestrictions`) are defined in `hdmi-hal` and re-exported here; they
are not duplicated.

The crate is traits-only. No implementations live here.

---

## Language feature: async fn in traits

MSRV is `1.85`, which is well above the `1.75` stabilisation of `async fn` in traits
(AFIT). The `async-trait` proc-macro crate is not needed.

AFIT-generated futures are not object-safe, which is the same trade-off `embedded-hal-async`
accepts. Trait objects are not a goal for this HAL layer.

Nor do the plain traits promise that their futures are `Send`. With concrete types the
compiler sees the real futures, so a training future over `Send` implementations is `Send`
and can be spawned on a multi-threaded executor. Code generic over `HdmiPhy` or
`ScdcTransport` cannot spawn one, however the types are bounded; it runs on a
single-threaded executor (Embassy, Tokio's current-thread runtime or `LocalSet`), uses
concrete types, or is bounded by the `Send` twins below.

---

## `Send` futures: the twins

`phy::SendHdmiPhy` and `scdc::SendScdcTransport` (from 0.2.1) have the same methods and
contract as the plain traits, each returning a future that is `Send`. This is the pattern
`trait_variant` generates, written by hand (no dependency):

```rust
pub trait SendHdmiPhy: Send {
    type Error: Send;
    fn set_frl_rate(&mut self, rate: HdmiForumFrl)
        -> impl Future<Output = Result<(), Self::Error>> + Send;
    // … one per method
}

impl<T: SendHdmiPhy> HdmiPhy for T {
    type Error = <T as SendHdmiPhy>::Error;
    fn set_frl_rate(&mut self, rate: HdmiForumFrl)
        -> impl Future<Output = Result<(), Self::Error>> {
        <T as SendHdmiPhy>::set_frl_rate(self, rate)
    }
    // …
}
```

- **Implementations write `async fn`**; the compiler checks that each future is `Send`.
- **`type Error: Send`**, unlike `trait_variant`: every user of a twin spawns, and a
  spawned task returns the errors, so the bound would otherwise be repeated by every
  caller (`P::Error: Send`).
- **`SendScdcTransport: Send + Sync`**: `read` takes `&self`, and a future holding `&self`
  is only `Send` when the transport is `Sync`. Interior state for reads therefore needs a
  `Sync` type (an atomic or a lock, not a `Cell`).
- **`read_block`**: required in both traits, and the blanket implementation forwards it, so
  a twin's `read_block` is used through either trait. `read_block_bytewise` serves both: a
  twin implementation is an `ScdcTransport` through the blanket implementation, and the
  helper's future is `Send` for any `T: SendScdcTransport` (a test proves it with a twin
  generic over another).
- **Blanket implementations**: every twin implementation is a plain-trait implementation
  too, so it works with code written against `HdmiPhy` or `ScdcTransport`. A type
  implements one of the two; implementing both conflicts with the blanket implementation
  (E0119). Existing implementations of the plain traits are unaffected.
- **Method calls**: a twin implementation has both traits' methods, so with both traits in
  scope a call on it is ambiguous (E0034). Import the trait the code is bounded by, or use
  the qualified form (`SendHdmiPhy::set_frl_rate(&mut phy, rate)`).
- **Bounds**: generic code is bounded by the twin alone. With `P: SendHdmiPhy + HdmiPhy`
  the compiler uses the explicit `HdmiPhy` bound rather than the blanket implementation,
  so the futures are opaque again and not known to be `Send` ("future cannot be sent
  between threads safely" on spawning); likewise `+ ScdcTransport`.

Return-type notation (`P: HdmiPhy<set_frl_rate(..): Send>`) would let callers ask for
`Send` futures of the plain traits without a second set of traits. It is not stable (rustc
1.99, October 2026); when it is, the twins are weighed against it. The tests show both
directions: a twin implementation used through the plain trait, and a function generic
over a twin whose future must be `Send` (it does not compile with the plain trait).

---

## Module structure

The module tree mirrors `hdmi-hal` exactly:

```
src/
  lib.rs          — #![no_std], module declarations, crate-level docs
  scdc/
    mod.rs        — async ScdcTransport trait and its Send twin, SendScdcTransport
  phy/
    mod.rs        — async HdmiPhy trait and its Send twin, SendHdmiPhy
```

---

## Traits

### `scdc::ScdcTransport`

Async version of `hdmi_hal::scdc::ScdcTransport`: the same contract, with the `fn`
keywords becoming `async fn`, and two clauses only async code needs: calls polled
together from one task (below) and cancellation.

```rust
pub trait ScdcTransport {
    type Error;

    async fn read(&self, reg: u8) -> Result<u8, Self::Error>;
    async fn write(&mut self, reg: u8, value: u8) -> Result<(), Self::Error>;

    /// One combined transfer for the whole buffer, or an error. Required.
    async fn read_block(&self, reg: u8, buf: &mut [u8]) -> Result<(), Self::Error>;
}

/// One `read` per byte: for a transport that cannot burst-read.
pub async fn read_block_bytewise<T: ScdcTransport + ?Sized>(
    transport: &T,
    reg: u8,
    buf: &mut [u8],
) -> Result<(), T::Error> { ... }
```

Doc comments mirror the sync version verbatim, with a note that the trait is the async
counterpart. That includes `read_block`'s contract (one combined transfer to the layer
below or an error, never split, a bus driver known to split it named; no snapshot
promised; a length limit, if any, documented and of at least 16 bytes; an empty buffer is
`Ok` with no bus activity; on error the buffer is unspecified; past `0xFF` unspecified
but never a panic) and the trait's "Errors" (usable after an error, or recovery
documented), restated rather than linked, since this crate's docs must stand on their
own for whichever hdmi-hal version it depends on.

One clause is added to "One transaction". Calls made at the same time through shared
references do not interleave on the bus, and in async code such calls can come from one
task: `join!` polls two futures on the same transport, which needs no `Sync`. A
transport keeping its bus in a `RefCell` and borrowing it across an `.await` panics on
the second borrow. The contract says an implementation serializes concurrent calls
(waits for the bus) and never panics; one that cannot returns an error for the second
call, and documents it. As in hdmi-hal, `read_block` has
no default: a per-byte default would let a transport whose author never considered block
reads tear related registers unnoticed; `read_block_bytewise` makes that choice explicit.

#### Cancellation

The one contract the sync trait does not need: an async call can be dropped before it
completes (a timeout, `select!`), and a sync one cannot. The trait documentation says:

- a dropped `read` returns nothing, and whether its transaction reached the bus is
  unknown; after a dropped `read_block`, the contents of `buf` are unspecified;
- after a dropped `write`, the write may or may not have reached the sink;
- an implementation **should** leave the transport usable for the next call, recovering
  the bus itself if a transfer was cut short; one that cannot **must** document what a
  cancelled call leaves behind and how to recover;
- a caller that needs to stop early checks its deadline before each call instead of
  dropping one in flight. This is the rule plumbob-async already gives for link training,
  made the default for every consumer: no transaction is cut short, whatever the
  transport does.

"Should", not "must", because the async I²C drivers a backend is built on differ, and a
requirement a backend cannot meet only makes it non-conforming; saying nothing would hide
the problem instead. What those drivers do with a dropped in-flight transfer (surveyed
2026-10-19; for the three embassy files, the copy read matched `main`'s blob hash):

| Source | A dropped in-flight transfer | Usable after? |
|---|---|---|
| `embedded-hal-async` 1.0.0, `I2c` | Not addressed; the transaction contract covers completed transfers only | unspecified |
| embassy-rp `src/i2c.rs` (embassy `77f1ee5d3848`) | A drop guard aborts; the next transfer waits for the controller to disable, discards the stale abort, and clears a stuck SDA (up to 9 SCL pulses, then a STOP) | yes, by design |
| esp-hal `src/i2c/master/mod.rs` (esp-hal `98256807a690`) | Documented: "aborts the transfer, but blocks while the driver finishes clearing and releasing the bus" (cleanup code not seen) | yes, per its docs |
| embassy-stm32 `src/i2c/v2.rs` (embassy `08332c1b46a2`) | A drop guard clears DMA and interrupt enables and error flags; resets the peripheral only on bus or arbitration errors; sends no STOP | unclear |
| embassy-nrf `src/twim.rs` (embassy `14c79d365b64`) | No drop guard in the async transaction (the driver's `Drop` has `// TODO: check for abort`); RX DMA targets the caller's buffer, so it can outlive the future | no |
| Linux, async over the blocking ioctl | The ioctl cannot be cancelled and completes in the background (reasoning, not a source) | yes |

embassy-rp's recover-on-the-next-call is the model for a backend. Two more notes for
backend authors:

- A backend over a blocking API (`spawn_blocking`) reads into a buffer it owns and copies
  into `buf`: `&mut [u8]` cannot be lent to another thread.
- The embassy-nrf DMA hazard is the driver's soundness, which a trait cannot fix. A
  buffer the backend owns is not enough if it is local to the `async fn`: it is freed
  with the dropped future while DMA may still write it. A backend over such a driver
  reads into storage that outlives any in-flight transfer (a field of the transport, or
  a `'static` buffer), copies into `buf` once the transfer completes, and has the next
  call wait for or stop the previous DMA before reusing that storage.

### `phy::HdmiPhy`

Async version of `hdmi_hal::phy::HdmiPhy`. Same methods, same parameter types,
same semantics.

```rust
pub trait HdmiPhy {
    type Error;

    async fn set_frl_rate(&mut self, rate: HdmiForumFrl) -> Result<(), Self::Error>;
    async fn send_ltp(&mut self, patterns: LanePatterns) -> Result<(), Self::Error>;
    async fn set_frl_output(&mut self, output: FrlOutput) -> Result<(), Self::Error>;
    async fn adjust_equalization(&mut self, params: EqParams) -> Result<(), Self::Error>;
    async fn set_scrambling(&mut self, enabled: bool) -> Result<(), Self::Error>;
}
```

`HdmiForumFrl` comes from `display-types` via re-export in `hdmi-hal`.
`LanePatterns`, `LtpPattern`, `FrlOutput` and `EqParams` are re-exported from `hdmi-hal`
(see below).

**Not covered yet.** As in `hdmi-hal`, today's surface is what link training needs. Lane
mapping, TMDS pre-emphasis and swing, spread-spectrum clocking and power states are planned
for when the mode-setting layer needs them. They are added to `hdmi-hal` first and mirrored
here in the same release; `doc/roadmap.md` tracks them.

---

## Re-exports

Data types that live in `hdmi-hal` and are used in trait signatures are re-exported, so
callers need no direct `hdmi-hal` dependency. They are available at two paths, as in
`hdmi-hal`'s `phy` module and at the crate root:

```rust
// src/phy/mod.rs: the same paths as `hdmi_hal::phy::*`
pub use hdmi_hal::phy::{
    EqParams, FrlOutput, LaneEqParams, LanePatterns, LtpPattern, TxFfeLevel, TxFfeRestrictions,
};
// src/lib.rs
pub use hdmi_hal::phy::{
    EqParams, FrlOutput, LaneEqParams, LanePatterns, LtpPattern, TxFfeLevel, TxFfeRestrictions,
};
```

Code moving between the sync and async traits can keep its `phy::` imports.

`HdmiForumFrl`, which `HdmiPhy::set_frl_rate` takes, is re-exported at the crate root as
well (`hdmi_hal_async::HdmiForumFrl`, from `hdmi-hal`), so a PHY implementation needs no
direct `display-types` dependency to name it.

---

## Cargo.toml

`hdmi-hal` (for the shared data types) and `display-types` (for `HdmiForumFrl` in the
`HdmiPhy` signature) are the only dependencies. No other dependencies are required. The
crate must remain `#![no_std]`: dependencies are declared with `default-features = false`,
and CI builds the crate for `thumbv7em-none-eabi`, a target without `std`.

---

## What this crate does not do

- **No implementations.** Platform backends (Embassy drivers, async kernel adaptors)
  implement the traits here; implementations do not live in this crate.
- **No sync-to-async adapters.** A wrapper that makes a sync `ScdcTransport` implement
  the async trait (by blocking) would belong in a compatibility shim crate, not here.
  The same split exists between `embedded-hal` and `embedded-hal-async`.
- **No executor dependency.** The traits impose no constraint on which async runtime is
  used; that is entirely the implementor's concern.
- **No plumbob or culvert dependency.** Those are higher-level protocol crates. The
  dependency only flows downward: protocol crates depend on HAL crates, not the reverse.

---

## Testing

Unit tests for a traits-only crate are minimal but still useful. Each trait module has a
`#[cfg(test)]` block with a mock implementor, mirroring `hdmi-hal`: `MockTransport` in
`scdc` and `MockPhy` in `phy`. That the mocks compile shows the trait signatures are
well-formed and that `async fn` in traits works end to end; the tests then check the
calls round-trip (register writes read back, `read_block` through `read_block_bytewise`
on the plain trait, on a twin, on a twin generic over another, and on an unsized
transport, each PHY call recorded).

The tests run on `pollster` (`#[pollster::test]`), a minimal blocking executor. It is a
dev-dependency only, so the library stays `no_std` and free of any executor.

---

## Release integrity

Every release is published through a GitHub Actions workflow that signs the `.crate`
package with [SLSA Build Level 2](https://slsa.dev) provenance. Verify with:

```sh
gh attestation verify hdmi-hal-async-X.Y.Z.crate --repo DracoWhitefire/hdmi-hal-async
```

---

## Consumers

| Crate           | Async companion of | Uses                                                                                                                           |
|-----------------|--------------------|--------------------------------------------------------------------------------------------------------------------------------|
| `culvert-async` | `culvert`          | `hdmi_hal_async::scdc::ScdcTransport`                                                                                          |
| `plumbob-async` | `plumbob`          | `hdmi_hal_async::phy::HdmiPhy`; SCDC through its own `ScdcClient` trait, which `culvert-async` implements over `ScdcTransport` |

`plumbob-async` has no state machine of its own: it drives plumbob's (`plumbob::lts::run`,
an `async fn`) with the async SCDC client and PHY, so training yields to the executor
whenever an SCDC or PHY call does.
