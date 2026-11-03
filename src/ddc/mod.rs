//! The DDC bus, async: one trait for every target on it.
//!
//! The async companion to `hdmi-hal`'s `ddc` module: [`DdcBus`] has the same contract as
//! `hdmi_hal::ddc::DdcBus`, with `async fn` operations, and [`SendDdcBus`] is its `Send`
//! twin. The vocabulary — [`DdcError`], [`DdcErrorKind`], [`NoAcknowledgeSource`],
//! [`EddcTarget`], [`DdcCapabilities`] and the bus's addresses — is `hdmi-hal`'s,
//! re-exported here, so an implementation names the same types either crate does.

pub use hdmi_hal::ddc::{
    DDC_CI, DISPLAYID, DdcCapabilities, DdcError, DdcErrorKind, EDID, EddcTarget, HDCP_PRIMARY,
    HDCP_SECONDARY, MAX_SEGMENT, NoAcknowledgeSource, SCDC, SEGMENT_POINTER, is_usable_address,
};

/// Access to the DDC bus of one connector, async.
///
/// The contract is `hdmi_hal::ddc::DdcBus`'s: the operations and what each puts on the wire,
/// one call one transaction ([`read_segmented`](Self::read_segmented) may be split, each
/// piece an E-DDC transaction of its own), sharing through `&self` with the implementation
/// serializing concurrent calls, the argument checks before bus activity, the E-DDC
/// behaviour and the errors. Calls polled together from one task (`join!`) reach even an
/// implementation that is not `Sync`: it serializes them as well, waiting for the bus, and
/// never panics (a `RefCell` borrowed across an `.await` would); one that cannot serialize
/// returns an error for the second call and says so.
///
/// [`capabilities`](Self::capabilities) is a plain `fn`: it reports, it does not touch the
/// bus.
///
/// # Cancellation
///
/// Any of these futures may be dropped before it completes, for example by a deadline in
/// the caller. What a dropped call has done is then unknown: whether its transaction
/// reached the bus, whether a write reached the sink. After a dropped call that reads, the
/// contents of `buf` are unspecified, partly written if the implementation splits the read.
///
/// An implementation should leave the bus usable for the next call, recovering it itself if
/// a transfer was cut short (for example by clearing a held bus before the next
/// transaction). One that cannot must say so in its documentation: what a cancelled call
/// leaves behind, and how to recover.
///
/// A caller that needs to stop early can avoid the question: check its deadline or cancel
/// signal before each call and stop there, rather than dropping a future mid-flight.
///
/// # `Send`
///
/// The futures of `async fn` in a trait are not known to be `Send` in generic code. Code
/// that spawns work on a multi-threaded executor uses [`SendDdcBus`]; every `SendDdcBus`
/// is a `DdcBus` too.
pub trait DdcBus {
    /// The error type, classified by [`DdcError::kind`].
    type Error: DdcError;

    /// What this implementation can do; never more than it can.
    fn capabilities(&self) -> DdcCapabilities;

    /// Write `bytes` to `addr` in one transaction.
    async fn write(&self, addr: u8, bytes: &[u8]) -> Result<(), Self::Error>;

    /// Write `bytes` to `addr`, then read `buf.len()` bytes from it after a repeated START,
    /// in one transaction.
    async fn write_read(&self, addr: u8, bytes: &[u8], buf: &mut [u8]) -> Result<(), Self::Error>;

    /// Read `buf.len()` bytes from `addr` in one transaction, with no write before it.
    async fn read(&self, addr: u8, buf: &mut [u8]) -> Result<(), Self::Error>;

    /// Read `buf.len()` bytes of `target`'s structure from `segment`, starting at `offset`
    /// within it: an E-DDC read.
    async fn read_segmented(
        &self,
        target: EddcTarget,
        segment: u8,
        offset: u8,
        buf: &mut [u8],
    ) -> Result<(), Self::Error>;
}

/// [`DdcBus`] with futures that are `Send`, for code that spawns work on a multi-threaded
/// executor; errors are `Send` too, since a spawned task returns them. Implementations
/// still write `async fn`; the compiler checks that each future is `Send`.
///
/// The bus is `Sync` as well as `Send`: every method takes `&self`, and a future holding
/// `&self` is only `Send` when `Self` is `Sync`. A bus that keeps interior state uses a
/// `Sync` type for it (an atomic or a lock rather than a `Cell`).
///
/// Every `SendDdcBus` is a [`DdcBus`], through a blanket implementation; a type implements
/// one of the two. With both traits in scope a method call on such a type is ambiguous:
/// import only the trait the code is bounded by, or call `SendDdcBus::write(…)`. Bound
/// generic code by `SendDdcBus` alone: adding `+ DdcBus` makes the compiler use that bound
/// instead of the blanket implementation, and the futures are no longer known to be `Send`.
///
/// There is no forwarding implementation for `&B`: next to the blanket implementation it
/// would conflict, since another crate may implement `SendDdcBus` for a reference to its
/// own type. Users share a bus through handles instead (`&B`, `Arc<B>`), as
/// [`ScdcOverDdc`](crate::scdc::ScdcOverDdc) does.
pub trait SendDdcBus: Send + Sync {
    /// The error type, classified by [`DdcError::kind`].
    type Error: DdcError + Send;

    /// See [`DdcBus::capabilities`].
    fn capabilities(&self) -> DdcCapabilities;

    /// See [`DdcBus::write`].
    fn write(&self, addr: u8, bytes: &[u8])
    -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// See [`DdcBus::write_read`].
    fn write_read(
        &self,
        addr: u8,
        bytes: &[u8],
        buf: &mut [u8],
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// See [`DdcBus::read`].
    fn read(
        &self,
        addr: u8,
        buf: &mut [u8],
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// See [`DdcBus::read_segmented`].
    fn read_segmented(
        &self,
        target: EddcTarget,
        segment: u8,
        offset: u8,
        buf: &mut [u8],
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

impl<T: SendDdcBus> DdcBus for T {
    type Error = <T as SendDdcBus>::Error;

    fn capabilities(&self) -> DdcCapabilities {
        <T as SendDdcBus>::capabilities(self)
    }

    fn write(&self, addr: u8, bytes: &[u8]) -> impl Future<Output = Result<(), Self::Error>> {
        <T as SendDdcBus>::write(self, addr, bytes)
    }

    fn write_read(
        &self,
        addr: u8,
        bytes: &[u8],
        buf: &mut [u8],
    ) -> impl Future<Output = Result<(), Self::Error>> {
        <T as SendDdcBus>::write_read(self, addr, bytes, buf)
    }

    fn read(&self, addr: u8, buf: &mut [u8]) -> impl Future<Output = Result<(), Self::Error>> {
        <T as SendDdcBus>::read(self, addr, buf)
    }

    fn read_segmented(
        &self,
        target: EddcTarget,
        segment: u8,
        offset: u8,
        buf: &mut [u8],
    ) -> impl Future<Output = Result<(), Self::Error>> {
        <T as SendDdcBus>::read_segmented(self, target, segment, offset, buf)
    }
}

#[cfg(test)]
mod tests;
