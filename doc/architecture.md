# hdmi-hal-async — Architecture and Implementation Plan

## Overview

`hdmi-hal-async` is the async companion to `hdmi-hal`, following the same design split
as `embedded-hal` / `embedded-hal-async`. It defines async versions of the traits in
`hdmi-hal`:

- `ddc::DdcBus` — the connector's whole DDC bus: any 7-bit target, and E-DDC reads of EDID
  and DisplayID
- `scdc::ScdcTransport` — raw byte-level SCDC register access
- `phy::FrlPhy` — what FRL link training needs from the transmitter PHY: FRL rate, link
  training patterns, FRL output mode and each lane's TxFFE level
- `phy::TmdsPhy` — what TMDS bring-up and tear-down need from the transmitter PHY:
  character rate and clock ratio, the source's scrambler, starting and stopping the output
- `phy::PhyPower` — powering the PHY off and on, shared by both mode traits
- `time::Delay` — waiting, for the procedures that drive these traits; `time::Clock` and
  `time::Timestamp` are `hdmi-hal`'s, re-exported, since reading a clock does not wait

and a `Send` twin of each, `ddc::SendDdcBus`, `scdc::SendScdcTransport`, `phy::SendFrlPhy`,
`phy::SendTmdsPhy`, `phy::SendPhyPower` and `time::SendDelay`, for code that spawns on a
multi-threaded executor (see "`Send` futures: the twins" below).

Data types (`LtpPattern`, `LanePatterns`, `FrlOutput`, `EqParams`, `LaneEqParams`,
`TxFfeLevel`, `TxFfeRestrictions`, `TmdsClockRatio`, `TmdsPhyConfig`, `TmdsVideo`,
`TmdsPhyCapabilities`, `Timestamp`, and `ddc`'s `DdcError`, `DdcErrorKind`,
`NoAcknowledgeSource`, `EddcTarget`, `DdcCapabilities` and addresses) are defined in
`hdmi-hal` and re-exported here; they are not duplicated.

The crate holds traits, and the glue between its own traits: the twins' blanket
implementations, `read_block_bytewise`, and the adapters `ScdcOverDdc` and
`SendScdcOverDdc`. No hardware, platform or clock implementations, no policy.

---

## Language feature: async fn in traits

MSRV is `1.85`, which is well above the `1.75` stabilisation of `async fn` in traits
(AFIT). The `async-trait` proc-macro crate is not needed.

AFIT-generated futures are not object-safe, which is the same trade-off `embedded-hal-async`
accepts. Trait objects are not a goal for this HAL layer.

Nor do the plain traits promise that their futures are `Send`. With concrete types the
compiler sees the real futures, so a training future over `Send` implementations is `Send`
and can be spawned on a multi-threaded executor. Code generic over `FrlPhy` or
`ScdcTransport` cannot spawn one, however the types are bounded; it runs on a
single-threaded executor (Embassy, Tokio's current-thread runtime or `LocalSet`), uses
concrete types, or is bounded by the `Send` twins below.

---

## `Send` futures: the twins

Each twin (`ddc::SendDdcBus`, `phy::SendFrlPhy`, `phy::SendTmdsPhy`, `phy::SendPhyPower`,
`scdc::SendScdcTransport`, `time::SendDelay`) has the same methods and contract as its
plain trait, cancellation included, each returning a future that is `Send`. The first twins
came in 0.2.1, `SendFrlPhy` then named `SendHdmiPhy`. This is the pattern `trait_variant`
generates, written by hand (no dependency):

```rust
pub trait SendFrlPhy: Send {
    type Error: core::fmt::Debug + Send;
    fn set_frl_rate(&mut self, rate: HdmiForumFrl)
        -> impl Future<Output = Result<(), Self::Error>> + Send;
    // … one per method
}

impl<T: SendFrlPhy> FrlPhy for T {
    type Error = <T as SendFrlPhy>::Error;
    fn set_frl_rate(&mut self, rate: HdmiForumFrl)
        -> impl Future<Output = Result<(), Self::Error>> {
        <T as SendFrlPhy>::set_frl_rate(self, rate)
    }
    // …
}
```

- **Implementations write `async fn`**; the compiler checks that each future is `Send`.
- **`type Error: Send`**, unlike `trait_variant`: every user of a twin spawns, and a
  spawned task returns the errors, so the bound would otherwise be repeated by every
  caller (`P::Error: Send`). Where the plain trait bounds its error (the PHY traits'
  `Debug`, `DdcBus`'s and `ScdcTransport`'s `DdcError`), the twin adds `Send` to that
  bound. `SendDelay` has no error.
- **Plain reports stay plain functions** in the twin too: `SendTmdsPhy::capabilities` is a
  `fn`, forwarded by the blanket implementation.
- **`SendScdcTransport: Send + Sync`** and **`SendDdcBus: Send + Sync`**: `read` (every
  method, for the bus) takes `&self`, and a future holding `&self` is only `Send` when the
  type is `Sync`. Interior state therefore needs a `Sync` type (an atomic or a lock, not a
  `Cell`). The same holds for a `time::Clock` used by spawned code: it has no twin (`now`
  is not `async`), so such code bounds `C: Clock + Sync`.
- **`read_block`**: required in both traits, and the blanket implementation forwards it, so
  a twin's `read_block` is used through either trait. `read_block_bytewise` serves both: a
  twin implementation is an `ScdcTransport` through the blanket implementation, and the
  helper's future is `Send` for any `T: SendScdcTransport` (a test proves it with a twin
  generic over another).
- **Blanket implementations**: every twin implementation is a plain-trait implementation
  too, so it works with code written against `FrlPhy` or `ScdcTransport`. A type
  implements one of the two; implementing both conflicts with the blanket implementation
  (E0119). Existing implementations of the plain traits are unaffected.
- **Method calls**: a twin implementation has both traits' methods, so with both traits in
  scope a call on it is ambiguous (E0034). Import the trait the code is bounded by, or use
  the qualified form (`SendFrlPhy::set_frl_rate(&mut phy, rate)`).
- **Bounds**: generic code is bounded by the twin alone. With `P: SendFrlPhy + FrlPhy`
  the compiler uses the explicit `FrlPhy` bound rather than the blanket implementation,
  so the futures are opaque again and not known to be `Send` ("future cannot be sent
  between threads safely" on spawning); likewise `+ ScdcTransport`.

Return-type notation (`P: FrlPhy<set_frl_rate(..): Send>`) would let callers ask for
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
  ddc/
    mod.rs        — async DdcBus trait and its Send twin, SendDdcBus; hdmi-hal's vocabulary
    tests.rs      — the twin, sharing a bus that is not Send, the re-exports
  scdc/
    mod.rs        — async ScdcTransport trait and its Send twin, SendScdcTransport;
                    read_block_bytewise; the adapters ScdcOverDdc and SendScdcOverDdc
  phy/
    mod.rs        — async FrlPhy, TmdsPhy and PhyPower traits and their Send twins
  time/
    mod.rs        — async Delay trait and its Send twin; Clock and Timestamp re-exported
```

---

## Traits

### `scdc::ScdcTransport`

Async version of `hdmi_hal::scdc::ScdcTransport`: the same contract, with the `fn`
keywords becoming `async fn`, and two clauses only async code needs: calls polled
together from one task (below) and cancellation.

```rust
pub trait ScdcTransport {
    type Error: DdcError;

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

The error is bounded by `DdcError`, as in `hdmi-hal`, and the twin's by `DdcError + Send`:
every transport error carries a `DdcErrorKind`. Doc comments mirror the sync version
verbatim, with a note that the trait is the async counterpart. That includes `read_block`'s contract (one combined transfer to the layer
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

### `ddc::DdcBus`

Async version of `hdmi_hal::ddc::DdcBus`: one trait for the connector's DDC bus, with
`write`, `write_read` and `read` on any 7-bit address and `read_segmented` for E-DDC reads
of EDID and DisplayID. `capabilities` stays a plain `fn`: it reports, it does not touch the
bus.

```rust
pub trait DdcBus {
    type Error: DdcError;

    fn capabilities(&self) -> DdcCapabilities;
    async fn write(&self, addr: u8, bytes: &[u8]) -> Result<(), Self::Error>;
    async fn write_read(&self, addr: u8, bytes: &[u8], buf: &mut [u8]) -> Result<(), Self::Error>;
    async fn read(&self, addr: u8, buf: &mut [u8]) -> Result<(), Self::Error>;
    async fn read_segmented(
        &self,
        target: EddcTarget,
        segment: u8,
        offset: u8,
        buf: &mut [u8],
    ) -> Result<(), Self::Error>;
}
```

The contract is `hdmi-hal`'s, and `hdmi-hal`'s architecture document gives the reasons for
each part: what each operation puts on the wire; one call one transaction, with
`read_segmented` allowed to split into E-DDC transactions of its own; the argument checks
before any bus activity; the E-DDC behaviour; errors that carry a `DdcErrorKind`, with no
retries of the implementation's own; capabilities that never over-state. The trait
documentation restates it rather than linking, as for `ScdcTransport`. Two parts only
async code needs:

- **Calls polled together from one task.** Every method takes `&self`, so `join!` can poll
  two calls on one bus, which needs no `Sync`. The implementation serializes them, waiting
  for the bus, and never panics, as `ScdcTransport`'s contract already says; one that
  cannot serialize returns an error for the second call and says so.
- **Cancellation**, as for `ScdcTransport`: what a dropped call did is unknown, whether its
  transaction reached the bus or a write the sink. After a dropped call that reads, the
  contents of `buf` are unspecified, partly written if the implementation splits the read.
  An implementation should leave the bus usable for the next call, recovering it itself;
  one that cannot documents what a cancelled call leaves and how to recover. A caller that
  needs to stop early checks before each call instead.

**`SendDdcBus`** is the twin, `Send + Sync` with `type Error: DdcError + Send`. **There is
no forwarding implementation for `&B`**: next to the twin's blanket implementation it would
conflict (E0119), since `&` is a fundamental type and another crate may implement
`SendDdcBus` for a reference to its own type. Users hold a handle instead: code over the bus
is generic over `ddc::DdcHandle` (or `ddc::SendDdcHandle`, `Send + Sync`), implemented for
every handle that dereferences to such a bus, so a bus that is not `Send` (embassy's) is
shared through `&'static B` or `Rc<B>`, and a `SendDdcBus` through `&B` or `Arc<B>`; a
`Copy` view of a bus implements the handle traits for itself and is passed by value.

#### `ScdcOverDdc` and `SendScdcOverDdc`

`scdc::ScdcOverDdc<H>` is the async `ScdcTransport` over a `DdcBus`, with `hdmi-hal`'s
behaviour: `read` and `read_block` are one `write_read` each at 0x54 (`ddc::SCDC`, whose
documentation in `hdmi-hal` cites the sources for the address), `write` one `write`,
its errors are the bus's, and `new` and `bytewise` return the handle when the bus cannot
serve SCDC.

It cannot also implement `SendScdcTransport`: the twin's blanket implementation already
provides `ScdcTransport` for every `SendScdcTransport`, so one generic type implementing both
conflicts (E0119). Code bounded by the twins, such as a client generic over
`SendScdcTransport`, therefore takes **`scdc::SendScdcOverDdc<H>`**, which implements
`SendScdcTransport` (and through the blanket `ScdcTransport`) for a `SendDdcHandle`. This is the stack's "second type"
pattern. Which to use: `ScdcOverDdc` for plain callers, and for `Send` callers that see the
concrete type (over a `SendDdcBus`, its futures are `Send` wherever the compiler sees the
type); `SendScdcOverDdc` where code is bounded by the twins.

### `phy::FrlPhy`

Async version of `hdmi_hal::phy::FrlPhy`. Same methods, same parameter types, same
semantics: `set_frl_rate` with a rate enters FRL (leaving TMDS on a PHY that does both),
and `HdmiForumFrl::NotSupported` turns FRL off and enters no other mode, as
`TmdsPhy::stop_output` on a PHY in FRL does.

```rust
pub trait FrlPhy {
    type Error: core::fmt::Debug;

    async fn set_frl_rate(&mut self, rate: HdmiForumFrl) -> Result<(), Self::Error>;
    async fn send_ltp(&mut self, patterns: LanePatterns) -> Result<(), Self::Error>;
    async fn set_frl_output(&mut self, output: FrlOutput) -> Result<(), Self::Error>;
    async fn adjust_equalization(&mut self, params: EqParams) -> Result<(), Self::Error>;
}
```

`HdmiForumFrl` comes from `display-types` via re-export in `hdmi-hal`.
`LanePatterns`, `LtpPattern`, `FrlOutput` and `EqParams` are re-exported from `hdmi-hal`
(see below). Before the split by link mode the trait was `HdmiPhy` and carried
`set_scrambling`, which is the source's TMDS scrambler and moved to `TmdsPhy`.

**Cancellation.** A dropped call may or may not have applied its setting, or may be
part-way through it. Each method sets a whole state (the rate, the per-lane patterns, the
output mode, the equalization), so calling it again with the state wanted recovers; an
implementation for which that does not hold documents what a cancelled call leaves and how
to recover. A caller that checks its deadline or cancel signal between calls, as
plumbob-async does, never meets the question.

### `phy::TmdsPhy`

Async version of `hdmi_hal::phy::TmdsPhy`: every method `async` but `capabilities`, a
plain report of what the implementation already knows, neither `async` nor fallible.

```rust
pub trait TmdsPhy {
    type Error: core::fmt::Debug;

    fn capabilities(&self) -> TmdsPhyCapabilities;
    async fn configure(&mut self, config: TmdsPhyConfig) -> Result<(), Self::Error>;
    async fn set_scrambling(&mut self, enabled: bool) -> Result<(), Self::Error>;
    async fn start_output(&mut self) -> Result<(), Self::Error>;
    async fn stop_output(&mut self) -> Result<(), Self::Error>;
}
```

The contract is `hdmi-hal`'s, restated in the trait documentation: `configure` leaves the
output stopped, enters TMDS and leaves the scrambler state unspecified; `set_scrambling(false)` succeeds on a PHY without a
scrambler, and `set_scrambling(true)` on one returns an error; `start_output` completes
once the PHY transmits, since callers time from its completion, and returns an error
without a successful `configure` since the last power-on; `stop_output` is callable in any
state, keeps the PHY powered and its TMDS configuration, and on a PHY in FRL leaves FRL.
`configure` with the output running either
returns an error or stops it first, and on a powered-off PHY returns an error.

**Cancellation.** After a dropped `configure` the rate and ratio are unspecified, after a
dropped `set_scrambling` the scrambler state, after a dropped `start_output` or
`stop_output` whether the output runs. `stop_output`, `configure` and then `set_scrambling`
recover any of them (`configure` leaves the scrambler state unspecified, so the scrambler is
set again after it); an implementation that cannot says so.

### `phy::PhyPower`

Async version of `hdmi_hal::phy::PhyPower`, shared by both mode traits: `power_on` leaves
the output stopped and no mode entered, `power_off` leaves no mode, so a bring-up configures
again; both are callable in any state, and `power_on` on a PHY that is already on does
nothing. A trait of its own rather than a supertrait, so link
training and TMDS bring-up never need it and a PHY that is always on implements the mode
traits only.

```rust
pub trait PhyPower {
    type Error: core::fmt::Debug;

    async fn power_on(&mut self) -> Result<(), Self::Error>;
    async fn power_off(&mut self) -> Result<(), Self::Error>;
}
```

**Cancellation.** After a dropped call the power state is unknown; `power_off` again
recovers, since it is callable in any state.

**One type, several traits.** Generic code bounded by `TmdsPhy + PhyPower` names the error
through one trait and ties the other to it (`PhyPower<Error = <P as TmdsPhy>::Error>`), as
`PhyPower`'s documentation shows. A type implementing both `TmdsPhy` and `DdcBus` has two
`capabilities` methods, so with both traits in scope the call is qualified
(`TmdsPhy::capabilities(&p)`); one value rarely serves as both, so the names are kept.

### `time::Delay`

Async version of `hdmi_hal::time::Delay`: wait at least the duration given, rounding up to
the implementation's tick, never less, measured on the platform's `Clock` (`now()` after
`delay(d)` completes is at least `d` later than before it); infallible. `Clock` and `Timestamp` are
`hdmi-hal`'s, re-exported unchanged: reading a counter does not wait, so the clock is
neither `async` nor fallible in either crate, and one type serves sync and async code.

```rust
pub use hdmi_hal::time::{Clock, Timestamp};

pub trait Delay {
    async fn delay(&mut self, duration: core::time::Duration);
}
```

`Delay` is the stack's own trait rather than a bound on embedded-hal-async's `DelayNs`:
generic code bounded by `DelayNs` cannot produce a `Send` future, and that crate has no
`Send` twin, so a procedure spawned on a multi-threaded executor could not use it.
`SendDelay` is the twin. An implementation over a `DelayNs` is a few lines, shown in the
module documentation; no adapter type ships, which would add a dependency. A `SendDelay`
cannot be written generically over `DelayNs`, whose futures are not known to be `Send`; it
is implemented over a concrete timer whose future is (embassy-time's timer, tokio's
`sleep`).

**No forwarding implementations** for references, as for `DdcBus`: a `Delay` for `&mut D`
would conflict with `SendDelay`'s blanket implementation (E0119), and `hdmi-hal`'s `Clock`
and `Delay` keep the same shape. An implementation that wants to be shared implements the
traits for a reference to its own type. That is how one test double serves as clock and
delay when a procedure takes them as separate arguments (`&C` and `&mut D`): it keeps its
time in a `Cell` (an atomic for `Send` code) and implements `Delay` for `&Double`. Whether
the procedures take one value or two is settled with their types, in the TMDS bring-up
prototype.

**Cancellation.** A dropped delay has done nothing that needs undoing, so a caller may
race a delay against an event.

**Not covered yet.** As in `hdmi-hal`, the PHY surface is what link training and TMDS
bring-up need. Lane mapping, TMDS pre-emphasis and swing, spread-spectrum clocking and
power states beyond off and on are planned for when the mode-setting layer needs them,
including whether each belongs in the traits or in a backend's own configuration (TMDS swing
and pre-emphasis are the implementation's today, outside `TmdsPhy`). Trait additions go to
`hdmi-hal` first and mirrored here in the same release;
`doc/roadmap.md` tracks them.

---

## Re-exports

Data types that live in `hdmi-hal` and are used in trait signatures are re-exported, so
callers need no direct `hdmi-hal` dependency. They are available at two paths, as in
`hdmi-hal`'s `phy` module and at the crate root:

```rust
// src/phy/mod.rs: the same paths as `hdmi_hal::phy::*`
pub use hdmi_hal::phy::{
    EqParams, FrlOutput, LaneEqParams, LanePatterns, LtpPattern, TmdsClockRatio,
    TmdsPhyCapabilities, TmdsPhyConfig, TmdsVideo, TxFfeLevel, TxFfeRestrictions,
};
// src/lib.rs
pub use hdmi_hal::phy::{
    EqParams, FrlOutput, LaneEqParams, LanePatterns, LtpPattern, TmdsClockRatio,
    TmdsPhyCapabilities, TmdsPhyConfig, TmdsVideo, TxFfeLevel, TxFfeRestrictions,
};
// src/time/mod.rs: the same paths as `hdmi_hal::time::*`
pub use hdmi_hal::time::{Clock, Timestamp};
// src/ddc/mod.rs: the same paths as `hdmi_hal::ddc::*`
pub use hdmi_hal::ddc::{
    DDC_CI, DISPLAYID, DdcCapabilities, DdcError, DdcErrorKind, EDID, EddcTarget, HDCP_PRIMARY,
    HDCP_SECONDARY, MAX_SEGMENT, NoAcknowledgeSource, SCDC, SEGMENT_POINTER, is_usable_address,
};
```

Code moving between the sync and async traits can keep its `ddc::`, `phy::` and `time::`
imports.

`ColorBitDepth` and `ColorFormat`, which `TmdsVideo` carries, are re-exported from
`display-types` under `phy` and at the crate root, as in `hdmi-hal`, so a `TmdsPhy`
implementation needs no direct `display-types` dependency.

`HdmiForumFrl`, which `FrlPhy::set_frl_rate` takes, is re-exported at the crate root as
well (`hdmi_hal_async::HdmiForumFrl`, from `hdmi-hal`), so a PHY implementation needs no
direct `display-types` dependency to name it.

---

## Cargo.toml

`hdmi-hal` (for the shared data types) and `display-types` (for `HdmiForumFrl` in the
`FrlPhy` signature) are the only dependencies. No other dependencies are required; the
dev-dependencies (`pollster` for the tests, `embedded-hal-async` for the `Delay` example in
the `time` module's documentation) do not reach users. The
crate must remain `#![no_std]`: dependencies are declared with `default-features = false`,
and CI builds the crate for `thumbv7em-none-eabi`, a target without `std`.

---

## What this crate does not do

- **No platform implementations.** Platform backends (Embassy drivers, async kernel
  adaptors) implement the traits here; they do not live in this crate. The only
  implementations here are glue between its own traits: the twins' blanket
  implementations and the adapters from `DdcBus` to `ScdcTransport`.
- **No sync-to-async adapters.** A wrapper that makes a sync `ScdcTransport` implement
  the async trait (by blocking) would belong in a compatibility shim crate, not here.
  The same split exists between `embedded-hal` and `embedded-hal-async`.
- **No executor dependency.** The traits impose no constraint on which async runtime is
  used; that is entirely the implementor's concern.
- **No plumbob or culvert dependency.** Those are higher-level protocol crates. The
  dependency only flows downward: protocol crates depend on HAL crates, not the reverse.

---

## Testing

Unit tests for a crate of traits are minimal but still useful. Each trait module has a
`#[cfg(test)]` block with mock implementors, mirroring `hdmi-hal`: in `ddc`, a `Send` bus
used through both traits and a bus that is not `Send`, shared through references and `Rc`;
`MockTransport` and the adapters over a bus in `scdc`; `MockPhy`, `MockTmdsPhy` and a PHY implementing `TmdsPhy` and `PhyPower` with one
error type in `phy`; a clock and delay on one type in `time`, the delay advancing the
clock. Each twin has a mock that implements only the twin, driven through both traits,
with a generic function whose future must be `Send`. That the mocks compile shows the
trait signatures are
well-formed and that `async fn` in traits works end to end; the tests then check the
calls round-trip (register writes read back, `read_block` through `read_block_bytewise`
on the plain trait, on a twin, on a twin generic over another, and on an unsized
transport, each PHY and power call recorded, a deadline polled with the clock and
delay).

The tests run on `pollster` (`#[pollster::test]`), a minimal blocking executor. It is a
dev-dependency only, so the library stays `no_std` and free of any executor.

---

## Release integrity

Every release is published through a GitHub Actions workflow that signs the `.crate`
package with [SLSA Build Level 2](https://slsa.dev) provenance. Verify with:

```sh
gh attestation verify hdmi-hal-async-X.Y.Z.crate --repo ashlar-display/hdmi-hal-async
```

Releases from before the move to the `ashlar-display` organization verify against the
previous owner; see "Verifying releases" in the README.

---

## Consumers

| Crate           | Async companion of | Uses                                                                                                                           |
|-----------------|--------------------|--------------------------------------------------------------------------------------------------------------------------------|
| `culvert-async` | `culvert`          | `hdmi_hal_async::scdc::ScdcTransport`                                                                                          |
| `plumbob-async` | `plumbob`          | `hdmi_hal_async::phy::FrlPhy`; SCDC through its own `ScdcClient` trait, which `culvert-async` implements over `ScdcTransport` |

`plumbob-async` has no state machine of its own: it drives plumbob's (`plumbob::lts::run`,
an `async fn`) with the async SCDC client and PHY, so training yields to the executor
whenever an SCDC or PHY call does. It names `FrlPhy` from the release that renames
`HdmiPhy`. The async TMDS bring-up procedures (planned, the async companion of
`spiritlevel`) are the main users of `TmdsPhy`, `Delay` and `Clock`; the caller that owns
the link uses `PhyPower`.
