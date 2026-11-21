//! Async hardware abstraction traits for the HDMI stack.
//!
//! `hdmi-hal-async` is the async companion to `hdmi-hal`, following the same split as
//! `embedded-hal` / `embedded-hal-async`. It mirrors each trait in `hdmi-hal` with
//! `async fn` methods and otherwise identical signatures. Like `hdmi-hal`, it holds traits
//! and the glue between its own traits (the `Send` twins' blanket implementations,
//! [`scdc::read_block_bytewise`], [`scdc::ScdcOverDdc`], [`scdc::SendScdcOverDdc`]): no
//! hardware, platform or clock implementations, no policy.
//!
//! Each trait also has a `Send` twin, [`phy::SendFrlPhy`], [`phy::SendTmdsPhy`],
//! [`phy::SendPhyPower`], [`scdc::SendScdcTransport`], [`ddc::SendDdcBus`] and
//! [`time::SendDelay`], for code that spawns work on a multi-threaded executor: the same
//! methods, returning futures that are `Send`. Every twin implementation is an
//! implementation of the plain trait as well.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
// async fn in traits does not guarantee Send on the returned futures. This is intentional
// for the plain traits: they serve single-threaded embedded executors (e.g. Embassy), where
// Send is not required, the same decision embedded-hal-async made. The Send twins
// (SendFrlPhy, SendTmdsPhy, SendPhyPower, SendScdcTransport, SendDdcBus, SendDelay) are for multi-threaded executors.
#![allow(async_fn_in_trait)]

/// A flag that asks a procedure to stop, re-exported from `hdmi-hal`: `is_cancelled` is not
/// `async`, so the sync trait serves async procedures as it is. A flag shared with a task
/// on a multi-threaded executor is `Sync`, as `AtomicBool` is.
pub mod cancel {
    pub use hdmi_hal::cancel::{Cancel, NoCancel};
}

pub mod ddc;

/// PHY control traits.
pub mod phy;

/// SCDC register transport trait.
pub mod scdc;

pub mod time;

pub use hdmi_hal::phy::{
    EqParams, FrlOutput, LaneEqParams, LanePatterns, LtpPattern, TmdsClockRatio,
    TmdsPhyCapabilities, TmdsPhyConfig, TmdsVideo, TxFfeLevel, TxFfeRestrictions,
};

/// The colour depth and format `phy::TmdsVideo` carries; re-exported from `display-types`
/// so that implementations name the same types this crate does.
pub use display_types::{ColorBitDepth, ColorFormat};

/// The FRL rate, as `phy::FrlPhy::set_frl_rate` takes it; re-exported so that implementations
/// name the same type this crate does.
pub use hdmi_hal::HdmiForumFrl;

// Compiles and runs the README's examples as doctests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
