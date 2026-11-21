# Roadmap

What each release contains is in the [changelog](../CHANGELOG.md); this page is about
what comes next.

## Planned

Mirrored from `hdmi-hal`: each item lands there first and here in the same release, with
the same shape. See `hdmi-hal`'s
[`doc/roadmap.md`](https://github.com/ashlar-display/hdmi-hal/blob/main/doc/roadmap.md)
for the details.

- **PHY traits by link mode (breaking)** — `HdmiPhy` and `SendHdmiPhy` renamed `FrlPhy` and
  `SendFrlPhy`, with no alias, and `set_scrambling` moved to a new `TmdsPhy` with its
  `Send` twin `SendTmdsPhy`; `PhyPower` and `SendPhyPower`; the `time` module, with
  `hdmi-hal`'s `Timestamp` and `Clock` re-exported and an async `Delay` with a hand-written
  `Send` twin, `SendDelay`, as the other traits have (generic code bounded by
  embedded-hal-async's `DelayNs` cannot produce a `Send` future, and that crate has no
  `Send` twin); a cancellation clause for each, matching `ScdcTransport`'s. On the
  development branch, described in the [architecture](architecture.md), and being proven by
  prototypes of the TMDS bring-up procedures before the release fixes them.
- **`DdcBus` (breaking)** — the async `ddc::DdcBus` with its `Send` twin `SendDdcBus`
  (`Send + Sync`, since its methods take `&self`), the same contract as `hdmi-hal`'s, and
  the cancellation clause `ScdcTransport` has: an implementation stays usable after a dropped
  call or says it does not, and the contents of `buf` are then undefined. `capabilities()`
  is a plain `fn`. `scdc::ScdcOverDdc` implements `ScdcTransport` over it, and
  `scdc::SendScdcOverDdc` implements `SendScdcTransport`, for code bounded by the `Send`
  twins: one generic type cannot implement both (E0119). Both take a handle to the bus
  (`ddc::DdcHandle`, `ddc::SendDdcHandle`): anything that dereferences to it, or a `Copy`
  view that is its own handle. No forwarding impl of `DdcBus` for `&B`: next to the twin's
  blanket impl it would conflict (E0119), so sharing goes through the handle. The
  `DdcError` bound on `ScdcTransport::Error` and its `Send` twin, and re-exports of
  `hdmi-hal`'s `DdcError`, `DdcErrorKind`, `NoAcknowledgeSource`, `EddcTarget` and
  `DdcCapabilities`, as for the PHY types.
- **Further PHY configuration** — lane mapping, TMDS pre-emphasis and swing,
  spread-spectrum clocking and power states beyond off and on, each decided when the layer
  that needs it is built, in the traits or in a backend's own configuration (TMDS swing and
  pre-emphasis are the implementation's today, outside `TmdsPhy`). As trait methods with default implementations they are not
  breaking changes; as required methods they would be.
- **CEC traits** — message-level and bit-level, when CEC work begins.
