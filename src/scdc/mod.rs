use core::ops::Deref;

use crate::ddc::{DdcBus, DdcCapabilities, DdcError, SCDC, SendDdcBus};

/// Async raw register access to the SCDC register map over DDC/I²C.
///
/// This is the async counterpart to `hdmi_hal::scdc::ScdcTransport`, with the same
/// contract, restated here, and two clauses only async code needs: calls made at the same
/// time from one task (in [`read_block`](Self::read_block)'s "One transaction") and
/// [Cancellation](#cancellation).
///
/// SCDC is a bidirectional channel used for FRL link training initiation, scrambling
/// control, and CED (Character Error Detection) reporting. Any code that needs to touch
/// the SCDC register map does so through this trait.
///
/// The trait operates at the raw register level: a one-byte address and a one-byte
/// value. Typed register wrappers, named constants, and multi-register sequences belong
/// in the protocol crate, not here.
///
/// Every method is required. A transport that can read several registers in one DDC/I²C
/// transaction implements [`read_block`](Self::read_block) with that transaction; one
/// that cannot implements it with [`read_block_bytewise`], so the choice is visible in its
/// code. Here, a bus that can read only one register per transaction:
///
/// ```
/// use core::convert::Infallible;
/// use hdmi_hal_async::scdc::{ScdcTransport, read_block_bytewise};
///
/// /// A bus whose controller reads one register per transaction.
/// struct OneRegisterBus {
///     registers: [u8; 256], // stands in for the sink
/// }
///
/// impl ScdcTransport for OneRegisterBus {
///     type Error = Infallible;
///
///     async fn read(&self, reg: u8) -> Result<u8, Infallible> {
///         Ok(self.registers[reg as usize])
///     }
///
///     async fn write(&mut self, reg: u8, value: u8) -> Result<(), Infallible> {
///         self.registers[reg as usize] = value;
///         Ok(())
///     }
///
///     /// One register at a time: not one transaction.
///     async fn read_block(&self, reg: u8, buf: &mut [u8]) -> Result<(), Infallible> {
///         read_block_bytewise(self, reg, buf).await
///     }
/// }
/// ```
///
/// A simulated sink, a register array that holds still while it is read, conforms with a
/// plain copy of its registers into `buf` instead.
///
/// # Errors
///
/// After any method returns an error, an implementation should leave the transport usable
/// for the next call, recovering the bus itself if a transfer failed midway (for example a
/// held data line after a NACK). One that cannot says so in its documentation: what an
/// error leaves behind, and how to recover (for example, by constructing the transport
/// again).
///
/// # Cancellation
///
/// Any of these futures may be dropped before it completes, for example by a
/// deadline in the caller. What a dropped call has done is then unknown:
///
/// - a dropped [`read`](Self::read) returns no value, and whether its transaction reached
///   the bus is unknown;
/// - after a dropped [`read_block`](Self::read_block), the contents of `buf` are
///   unspecified;
/// - after a dropped [`write`](Self::write), the write may or may not have reached the
///   sink, so a caller that needs to know reads the register back.
///
/// An implementation should leave the transport usable for the next call, recovering
/// the bus itself if the transfer was cut short (for example by clearing a held bus
/// before the next transaction). One that cannot must say so in its documentation: what
/// a cancelled call leaves behind, and how to recover (for example, by constructing the
/// transport again).
///
/// A caller that needs to stop early can avoid the question: check the deadline or cancel
/// signal before each call and stop there, rather than dropping a future mid-flight. No
/// transaction is then cut short, whatever the transport does.
pub trait ScdcTransport {
    /// Error type returned by transport operations, classified by [`DdcError::kind`]
    /// (implemented for [`Infallible`](core::convert::Infallible), `()` and
    /// [`DdcErrorKind`](crate::ddc::DdcErrorKind)). Over a [`DdcBus`], [`ScdcOverDdc`] is the
    /// transport.
    type Error: DdcError;

    /// Read a single byte from the given SCDC register address.
    async fn read(&self, reg: u8) -> Result<u8, Self::Error>;

    /// Write a single byte to the given SCDC register address.
    async fn write(&mut self, reg: u8, value: u8) -> Result<(), Self::Error>;

    /// Read consecutive SCDC registers, starting at `reg`, into `buf`.
    ///
    /// Related registers, such as the status flags or the character error counters, are
    /// read together, in one transaction, so that the transport does not split them;
    /// whether the sink keeps them consistent during that transaction is the sink's
    /// (see "No snapshot promised"). The contract:
    ///
    /// - **What it reads.** `buf[i]` is register `reg + i`.
    /// - **One transaction.** The whole buffer is read as one combined transfer to the
    ///   layer below — on the DDC bus, the register offset written, a repeated START,
    ///   `buf.len()` bytes read, STOP — or an error is returned. An implementation never
    ///   splits the read into several transfers, and never falls back to one register at
    ///   a time. Where the layer below is an OS bus driver, the transfer is handed to it
    ///   whole; a driver known to split it on the wire is named in the transport's
    ///   documentation, as the bytewise exception is. Calls made at the same time through
    ///   shared references do not interleave on the bus. That includes calls polled
    ///   together from one task (`join!`), which reach even a transport that is not
    ///   `Sync`: an implementation serializes them, waiting for the bus, and never panics
    ///   (a `RefCell` borrowed across an `.await` would); one that cannot serialize returns
    ///   an error for the second call instead, and says so. [`read_block_bytewise`] is
    ///   the one exception to the first two rules, and a transport that uses it says so.
    /// - **No snapshot promised.** One transaction does not mean the sink returns a
    ///   consistent snapshot: whether it holds its registers still during a read is up to
    ///   the sink.
    /// - **Length limit.** An implementation may have a maximum length, if it documents
    ///   it, of at least 16 bytes; a longer buffer is an error, not split. A caller that
    ///   needs a longer read splits it itself — parts of up to 16 bytes fit every
    ///   conforming transport — knowing the parts are separate transactions.
    /// - **Empty buffer.** `Ok`, with no bus activity.
    /// - **On error,** the contents of `buf` are unspecified (see the trait's "Errors" for
    ///   the transport itself).
    /// - **Past `0xFF`.** The caller does not read past register `0xFF`. If it does, the
    ///   result is unspecified, but the call returns, `Ok` or an error, and never panics:
    ///   [`read_block_bytewise`] wraps to `0x00`; a real burst read leaves it to the sink.
    async fn read_block(&self, reg: u8, buf: &mut [u8]) -> Result<(), Self::Error>;
}

/// Implements [`ScdcTransport::read_block`] with one [`read`](ScdcTransport::read) per
/// byte, for a transport that cannot read several registers in one transaction.
///
/// This is the documented exception to `read_block`'s "one transaction" rule: each byte
/// is its own transaction, so a value the sink updates between two of them can be read
/// half before and half after the update, and another caller's transaction can land in
/// between. A transport that uses it should say so in its documentation.
///
/// Reads stop at the first error, which is returned. An empty buffer reads nothing. A
/// read past register `0xFF` wraps to `0x00`.
///
/// A [`SendScdcTransport`] uses it too: through the blanket implementation, the
/// transport is an `ScdcTransport`, and the returned future is `Send`.
pub async fn read_block_bytewise<T: ScdcTransport + ?Sized>(
    transport: &T,
    reg: u8,
    buf: &mut [u8],
) -> Result<(), T::Error> {
    let mut addr = reg;
    for byte in buf {
        *byte = transport.read(addr).await?;
        addr = addr.wrapping_add(1);
    }
    Ok(())
}

/// [`ScdcTransport`] with `Send` futures, for code that spawns link training on a
/// multi-threaded executor.
///
/// `ScdcTransport`'s `async fn`s promise nothing about their futures, so code generic over
/// `T: ScdcTransport` cannot spawn a future that calls them, however `T` is bounded. This
/// trait has the same methods and contract, each returning a future that is `Send`, and
/// errors that are `Send` too: a spawned task returns them. Implementations still write
/// `async fn`; the compiler checks that each future is `Send`.
///
/// The transport is `Sync` as well as `Send`: [`read`](Self::read) takes `&self`, and a
/// future holding `&self` is only `Send` when `Self` is `Sync`. A transport that keeps
/// interior state for reads uses a `Sync` type for it (an atomic or a lock rather than a
/// `Cell`).
///
/// Every `SendScdcTransport` is an `ScdcTransport`, through a blanket implementation, so
/// it works with everything written against `ScdcTransport` as well (its `read_block`
/// included); a type implements one of the two (implementing both conflicts with
/// the blanket implementation). A `SendScdcTransport` type has both traits' methods, so
/// with both traits in scope a method call on it is ambiguous: import only the trait the
/// code is bounded by, or call `SendScdcTransport::read(…)`.
///
/// Bound generic code by `SendScdcTransport` alone. Adding `+ ScdcTransport`, which it
/// implies anyway, makes the compiler use that bound instead of the blanket
/// implementation, and the futures are no longer known to be `Send`.
pub trait SendScdcTransport: Send + Sync {
    /// Error type returned by transport operations, classified by [`DdcError::kind`]. Over a
    /// [`SendDdcBus`], [`SendScdcOverDdc`] is the transport.
    type Error: DdcError + Send;

    /// See [`ScdcTransport::read`].
    fn read(&self, reg: u8) -> impl Future<Output = Result<u8, Self::Error>> + Send;

    /// See [`ScdcTransport::write`].
    fn write(&mut self, reg: u8, value: u8)
    -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// See [`ScdcTransport::read_block`], whose contract it has. A transport that cannot
    /// burst-read implements it with [`read_block_bytewise`]:
    ///
    /// ```
    /// use hdmi_hal_async::scdc::{SendScdcTransport, read_block_bytewise};
    /// # struct Backend;
    /// # impl SendScdcTransport for Backend {
    /// #     type Error = ();
    /// #     async fn read(&self, _reg: u8) -> Result<u8, ()> { Ok(0) }
    /// #     async fn write(&mut self, _reg: u8, _value: u8) -> Result<(), ()> { Ok(()) }
    /// async fn read_block(&self, reg: u8, buf: &mut [u8]) -> Result<(), ()> {
    ///     read_block_bytewise(self, reg, buf).await
    /// }
    /// # }
    /// ```
    fn read_block(
        &self,
        reg: u8,
        buf: &mut [u8],
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

impl<T: SendScdcTransport> ScdcTransport for T {
    type Error = <T as SendScdcTransport>::Error;

    fn read(&self, reg: u8) -> impl Future<Output = Result<u8, Self::Error>> {
        <T as SendScdcTransport>::read(self, reg)
    }

    fn write(&mut self, reg: u8, value: u8) -> impl Future<Output = Result<(), Self::Error>> {
        <T as SendScdcTransport>::write(self, reg, value)
    }

    fn read_block(&self, reg: u8, buf: &mut [u8]) -> impl Future<Output = Result<(), Self::Error>> {
        <T as SendScdcTransport>::read_block(self, reg, buf)
    }
}

/// Whether a bus with these capabilities can serve SCDC: reach it, write an offset and a
/// value, and read `min_read` registers after a one-byte offset in one `write_read`.
fn serves_scdc(caps: &DdcCapabilities, min_read: u16) -> bool {
    let (write_part, read_part) = caps.write_read_max();
    caps.reaches(SCDC) && caps.write_max() >= 2 && write_part >= 1 && read_part >= min_read
}

/// [`ScdcTransport`] over a [`DdcBus`]: the sink's SCDC at [`SCDC`] (0x54).
///
/// The async counterpart to `hdmi_hal::scdc::ScdcOverDdc`, with the same behaviour:
/// [`read`](ScdcTransport::read) and [`read_block`](ScdcTransport::read_block) are one
/// [`write_read`](DdcBus::write_read) each, [`write`](ScdcTransport::write) one
/// [`write`](DdcBus::write); `read_block`'s length limit is the bus's read limit for
/// `write_read`, and the bus refuses a longer read; constructed with
/// [`bytewise`](Self::bytewise), `read_block` reads one register per transaction
/// ([`read_block_bytewise`]). It holds a handle to the bus (`&B`, `Rc<B>`, `Arc<B>`), so
/// other users share the bus. Its errors are the bus's.
///
/// It implements [`ScdcTransport`] only. Code bounded by [`SendScdcTransport`] (for example
/// a client generic over the `Send` twins) takes [`SendScdcOverDdc`]: one generic type
/// cannot implement both traits, since `SendScdcTransport`'s blanket implementation
/// already provides `ScdcTransport`. Over a [`SendDdcBus`], this plain adapter's futures are
/// still `Send` wherever the compiler sees its concrete type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScdcOverDdc<H> {
    bus: H,
    bytewise: bool,
}

impl<H> ScdcOverDdc<H>
where
    H: Deref,
    H::Target: DdcBus,
{
    /// SCDC over `bus`, with `read_block` in one transaction; the handle back when the bus
    /// does not reach [`SCDC`], cannot write an offset and a value, or cannot read 16
    /// registers after a one-byte offset in one `write_read`.
    pub fn new(bus: H) -> Result<Self, H> {
        if serves_scdc(&bus.capabilities(), 16) {
            Ok(ScdcOverDdc {
                bus,
                bytewise: false,
            })
        } else {
            Err(bus)
        }
    }

    /// SCDC over `bus`, with `read_block` one register per transaction; the handle back when
    /// the bus does not reach [`SCDC`], cannot write an offset and a value, or cannot read
    /// one register after a one-byte offset.
    pub fn bytewise(bus: H) -> Result<Self, H> {
        if serves_scdc(&bus.capabilities(), 1) {
            Ok(ScdcOverDdc {
                bus,
                bytewise: true,
            })
        } else {
            Err(bus)
        }
    }

    /// Whether `read_block` reads one register per transaction.
    pub fn is_bytewise(&self) -> bool {
        self.bytewise
    }

    /// The bus handle.
    pub fn into_inner(self) -> H {
        self.bus
    }
}

impl<H> ScdcTransport for ScdcOverDdc<H>
where
    H: Deref,
    H::Target: DdcBus,
{
    type Error = <H::Target as DdcBus>::Error;

    async fn read(&self, reg: u8) -> Result<u8, Self::Error> {
        let mut value = [0u8];
        self.bus.write_read(SCDC, &[reg], &mut value).await?;
        Ok(value[0])
    }

    async fn write(&mut self, reg: u8, value: u8) -> Result<(), Self::Error> {
        self.bus.write(SCDC, &[reg, value]).await
    }

    async fn read_block(&self, reg: u8, buf: &mut [u8]) -> Result<(), Self::Error> {
        if self.bytewise {
            read_block_bytewise(self, reg, buf).await
        } else {
            self.bus.write_read(SCDC, &[reg], buf).await
        }
    }
}

/// [`SendScdcTransport`] over a [`SendDdcBus`]: [`ScdcOverDdc`] for code bounded by the
/// `Send` twins.
///
/// The same behaviour as [`ScdcOverDdc`], implementing [`SendScdcTransport`] (and through
/// its blanket implementation [`ScdcTransport`]), for a handle that is `Send + Sync` and
/// dereferences to a [`SendDdcBus`] (`&B`, `Arc<B>`). A client that requires
/// `SendScdcTransport` of its transport takes this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendScdcOverDdc<H> {
    bus: H,
    bytewise: bool,
}

impl<H> SendScdcOverDdc<H>
where
    H: Deref + Send + Sync,
    H::Target: SendDdcBus,
{
    /// See [`ScdcOverDdc::new`].
    pub fn new(bus: H) -> Result<Self, H> {
        if serves_scdc(&SendDdcBus::capabilities(&*bus), 16) {
            Ok(SendScdcOverDdc {
                bus,
                bytewise: false,
            })
        } else {
            Err(bus)
        }
    }

    /// See [`ScdcOverDdc::bytewise`].
    pub fn bytewise(bus: H) -> Result<Self, H> {
        if serves_scdc(&SendDdcBus::capabilities(&*bus), 1) {
            Ok(SendScdcOverDdc {
                bus,
                bytewise: true,
            })
        } else {
            Err(bus)
        }
    }

    /// Whether `read_block` reads one register per transaction.
    pub fn is_bytewise(&self) -> bool {
        self.bytewise
    }

    /// The bus handle.
    pub fn into_inner(self) -> H {
        self.bus
    }
}

impl<H> SendScdcTransport for SendScdcOverDdc<H>
where
    H: Deref + Send + Sync,
    H::Target: SendDdcBus,
{
    type Error = <H::Target as SendDdcBus>::Error;

    async fn read(&self, reg: u8) -> Result<u8, Self::Error> {
        let mut value = [0u8];
        SendDdcBus::write_read(&*self.bus, SCDC, &[reg], &mut value).await?;
        Ok(value[0])
    }

    async fn write(&mut self, reg: u8, value: u8) -> Result<(), Self::Error> {
        SendDdcBus::write(&*self.bus, SCDC, &[reg, value]).await
    }

    async fn read_block(&self, reg: u8, buf: &mut [u8]) -> Result<(), Self::Error> {
        if self.bytewise {
            read_block_bytewise(self, reg, buf).await
        } else {
            SendDdcBus::write_read(&*self.bus, SCDC, &[reg], buf).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;
    use core::sync::atomic::{AtomicU32, Ordering};

    /// A failed read, carrying its register.
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Fail(u8);

    impl DdcError for Fail {
        fn kind(&self) -> crate::ddc::DdcErrorKind {
            crate::ddc::DdcErrorKind::Other
        }
    }

    struct MockTransport {
        registers: [u8; 256],
        reads: Cell<u32>,
        fail_at: Option<u8>,
    }

    impl MockTransport {
        fn new() -> Self {
            Self {
                registers: [0u8; 256],
                reads: Cell::new(0),
                fail_at: None,
            }
        }

        /// A transport whose every register holds its own address.
        fn filled() -> Self {
            let mut t = Self::new();
            for (i, r) in t.registers.iter_mut().enumerate() {
                *r = i as u8;
            }
            t
        }
    }

    impl ScdcTransport for MockTransport {
        type Error = Fail;

        async fn read(&self, reg: u8) -> Result<u8, Self::Error> {
            if self.fail_at == Some(reg) {
                return Err(Fail(reg));
            }
            self.reads.set(self.reads.get() + 1);
            Ok(self.registers[reg as usize])
        }

        async fn write(&mut self, reg: u8, value: u8) -> Result<(), Self::Error> {
            self.registers[reg as usize] = value;
            Ok(())
        }

        async fn read_block(&self, reg: u8, buf: &mut [u8]) -> Result<(), Self::Error> {
            read_block_bytewise(self, reg, buf).await
        }
    }

    #[pollster::test]
    async fn write_then_read_returns_written_value() {
        let mut t = MockTransport::new();
        t.write(0x20, 0xAB).await.unwrap();
        assert_eq!(t.read(0x20).await.unwrap(), 0xAB);
    }

    #[pollster::test]
    async fn unwritten_register_reads_zero() {
        let t = MockTransport::new();
        assert_eq!(t.read(0x31).await.unwrap(), 0x00);
    }

    #[pollster::test]
    async fn write_is_register_specific() {
        let mut t = MockTransport::new();
        t.write(0x01, 0xFF).await.unwrap();
        assert_eq!(t.read(0x00).await.unwrap(), 0x00);
        assert_eq!(t.read(0x02).await.unwrap(), 0x00);
    }

    #[pollster::test]
    async fn write_overwrites_previous_value() {
        let mut t = MockTransport::new();
        t.write(0x20, 0x01).await.unwrap();
        t.write(0x20, 0x02).await.unwrap();
        assert_eq!(t.read(0x20).await.unwrap(), 0x02);
    }

    #[pollster::test]
    async fn bytewise_reads_consecutive_registers() {
        let t = MockTransport::filled();
        let mut buf = [0u8; 3];
        t.read_block(0x40, &mut buf).await.unwrap();
        assert_eq!(buf, [0x40, 0x41, 0x42]);
        assert_eq!(t.reads.get(), 3);
    }

    #[pollster::test]
    async fn bytewise_empty_buffer_reads_nothing() {
        let t = MockTransport::filled();
        t.read_block(0x40, &mut []).await.unwrap();
        assert_eq!(t.reads.get(), 0);
    }

    #[pollster::test]
    async fn bytewise_stops_at_first_error() {
        let mut t = MockTransport::filled();
        t.fail_at = Some(0x41);
        let mut buf = [0u8; 3];
        assert_eq!(t.read_block(0x40, &mut buf).await, Err(Fail(0x41)));
        assert_eq!(buf, [0x40, 0x00, 0x00]);
        assert_eq!(t.reads.get(), 1);
    }

    #[pollster::test]
    async fn bytewise_wraps_past_0xff() {
        let t = MockTransport::filled();
        let mut buf = [0u8; 2];
        t.read_block(0xFF, &mut buf).await.unwrap();
        assert_eq!(buf, [0xFF, 0x00]);
    }

    #[pollster::test]
    async fn write_then_read_block() {
        let mut t = MockTransport::new();
        t.write(0x20, 0xAB).await.unwrap();
        let mut buf = [0u8; 1];
        t.read_block(0x20, &mut buf).await.unwrap();
        assert_eq!(buf, [0xAB]);
    }

    /// Implements only the `Send` twin, with `read_block` through `read_block_bytewise`.
    /// Counts reads in an atomic, since the twin needs `Sync`.
    struct SendMockTransport {
        registers: [u8; 256],
        reads: AtomicU32,
        fail_at: Option<u8>,
    }

    impl SendMockTransport {
        fn filled() -> Self {
            let mut registers = [0u8; 256];
            for (i, r) in registers.iter_mut().enumerate() {
                *r = i as u8;
            }
            Self {
                registers,
                reads: AtomicU32::new(0),
                fail_at: None,
            }
        }

        fn reads(&self) -> u32 {
            self.reads.load(Ordering::Relaxed)
        }
    }

    impl SendScdcTransport for SendMockTransport {
        type Error = Fail;

        async fn read(&self, reg: u8) -> Result<u8, Self::Error> {
            if self.fail_at == Some(reg) {
                return Err(Fail(reg));
            }
            self.reads.fetch_add(1, Ordering::Relaxed);
            Ok(self.registers[reg as usize])
        }

        async fn write(&mut self, reg: u8, value: u8) -> Result<(), Self::Error> {
            self.registers[reg as usize] = value;
            Ok(())
        }

        async fn read_block(&self, reg: u8, buf: &mut [u8]) -> Result<(), Self::Error> {
            read_block_bytewise(self, reg, buf).await
        }
    }

    /// A `Send` twin generic over another, with `read_block` through `read_block_bytewise`:
    /// it compiles only if the helper's future is `Send` for every `T: SendScdcTransport`,
    /// not only for a concrete type.
    struct Forwarding<T>(T);

    impl<T: SendScdcTransport> SendScdcTransport for Forwarding<T> {
        type Error = T::Error;

        async fn read(&self, reg: u8) -> Result<u8, Self::Error> {
            SendScdcTransport::read(&self.0, reg).await
        }

        async fn write(&mut self, reg: u8, value: u8) -> Result<(), Self::Error> {
            SendScdcTransport::write(&mut self.0, reg, value).await
        }

        async fn read_block(&self, reg: u8, buf: &mut [u8]) -> Result<(), Self::Error> {
            read_block_bytewise(self, reg, buf).await
        }
    }

    /// Implements only the `Send` twin with its own `read_block`, as a transport that
    /// reads a block in one transaction does.
    struct BlockReadingTransport;

    impl SendScdcTransport for BlockReadingTransport {
        type Error = ();

        async fn read(&self, _reg: u8) -> Result<u8, Self::Error> {
            Err(())
        }

        async fn write(&mut self, _reg: u8, _value: u8) -> Result<(), Self::Error> {
            Err(())
        }

        async fn read_block(&self, reg: u8, buf: &mut [u8]) -> Result<(), Self::Error> {
            buf.fill(reg);
            Ok(())
        }
    }

    /// Writes, then reads back one register and a block, through `ScdcTransport`, as code
    /// written against the plain trait does.
    async fn round_trip_plain<T: ScdcTransport>(
        t: &mut T,
        buf: &mut [u8],
    ) -> (
        Result<(), T::Error>,
        Result<u8, T::Error>,
        Result<(), T::Error>,
    ) {
        let written = t.write(0x20, 0xAB).await;
        let read = t.read(0x20).await;
        let block = t.read_block(0x20, buf).await;
        (written, read, block)
    }

    /// The compiler proves this `Send` bound for every `T: SendScdcTransport`, not only
    /// for the mock; with `ScdcTransport` in its place it would not compile.
    fn spawnable<'a, T: SendScdcTransport>(
        t: &'a T,
        buf: &'a mut [u8],
    ) -> impl Future<Output = Result<(), T::Error>> + Send + 'a {
        SendScdcTransport::read_block(t, 0x40, buf)
    }

    fn assert_send<T: Send>(value: T) -> T {
        value
    }

    #[pollster::test]
    async fn send_twin_is_usable_as_scdc_transport() {
        let mut t = SendMockTransport::filled();
        let mut buf = [0u8; 2];
        let (written, read, block) = round_trip_plain(&mut t, &mut buf).await;
        assert_eq!((written, read, block), (Ok(()), Ok(0xAB), Ok(())));
        assert_eq!(buf, [0xAB, 0x21]);
        assert_eq!(t.reads(), 3);
    }

    #[pollster::test]
    async fn send_twin_futures_are_send_in_generic_code() {
        let t = SendMockTransport::filled();
        let mut buf = [0u8; 3];
        assert_send(spawnable(&t, &mut buf)).await.unwrap();
        assert_eq!(buf, [0x40, 0x41, 0x42]);
    }

    #[pollster::test]
    async fn bytewise_through_a_generic_send_twin() {
        let mut t = Forwarding(SendMockTransport::filled());
        let mut buf = [0u8; 2];
        assert_send(spawnable(&t, &mut buf)).await.unwrap();
        assert_eq!(buf, [0x40, 0x41]);
        assert_eq!(t.0.reads(), 2);
        // And through the plain trait, as code written against `ScdcTransport` uses it.
        let (written, read, block) = round_trip_plain(&mut t, &mut buf).await;
        assert_eq!((written, read, block), (Ok(()), Ok(0xAB), Ok(())));
        assert_eq!(buf, [0xAB, 0x21]);
    }

    #[pollster::test]
    async fn bytewise_on_an_unsized_transport() {
        // `?Sized`: `ScdcTransport` has `async fn`s, so it has no `dyn` form, but a
        // transport can still be unsized, here one whose registers are a slice.
        struct Regs<R: ?Sized>(R);
        impl ScdcTransport for Regs<[u8]> {
            type Error = ();
            async fn read(&self, reg: u8) -> Result<u8, ()> {
                self.0.get(reg as usize).copied().ok_or(())
            }
            async fn write(&mut self, _reg: u8, _value: u8) -> Result<(), ()> {
                Err(())
            }
            async fn read_block(&self, reg: u8, buf: &mut [u8]) -> Result<(), ()> {
                read_block_bytewise(self, reg, buf).await
            }
        }
        let t: &mut Regs<[u8]> = &mut Regs([0x10, 0x11, 0x12]);
        let mut buf = [0u8; 2];
        read_block_bytewise(t, 0x01, &mut buf).await.unwrap();
        assert_eq!(buf, [0x11, 0x12]);
        assert_eq!(t.read_block(0x02, &mut buf).await, Err(()));
        assert_eq!(t.write(0x00, 0xAB).await, Err(()));
    }

    #[pollster::test]
    async fn send_twin_bytewise_stops_at_first_error() {
        let mut t = SendMockTransport::filled();
        t.fail_at = Some(0x41);
        let mut buf = [0u8; 3];
        assert_eq!(
            SendScdcTransport::read_block(&t, 0x40, &mut buf).await,
            Err(Fail(0x41))
        );
        assert_eq!(buf, [0x40, 0x00, 0x00]);
        assert_eq!(t.reads(), 1);
    }

    #[pollster::test]
    async fn send_twin_bytewise_wraps_past_0xff() {
        let t = SendMockTransport::filled();
        let mut buf = [0u8; 2];
        SendScdcTransport::read_block(&t, 0xFF, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, [0xFF, 0x00]);
    }

    #[pollster::test]
    async fn send_twin_read_block_is_used_through_scdc_transport() {
        let mut t = BlockReadingTransport;
        let mut buf = [0u8; 2];
        let (written, read, block) = round_trip_plain(&mut t, &mut buf).await;
        assert_eq!((written, read, block), (Err(()), Err(()), Ok(())));
        assert_eq!(buf, [0x20, 0x20]);
    }

    extern crate alloc;
    use crate::ddc::{DdcErrorKind, EddcTarget};
    use alloc::rc::Rc;
    use alloc::sync::Arc;
    use core::sync::atomic::AtomicU8;

    /// A `Send` DDC bus over an SCDC register file, counting transactions.
    struct Bus {
        caps: DdcCapabilities,
        registers: [AtomicU8; 256],
        write_reads: AtomicU32,
        fail: Option<DdcErrorKind>,
    }

    impl Bus {
        fn scdc(read_limit: u16) -> Self {
            Self::with(
                DdcCapabilities::none()
                    .with_address_reachable(SCDC, true)
                    .with_write_max(2)
                    .with_write_read_max(1, read_limit),
            )
        }

        fn with(caps: DdcCapabilities) -> Self {
            Bus {
                caps,
                registers: core::array::from_fn(|i| AtomicU8::new(i as u8)),
                write_reads: AtomicU32::new(0),
                fail: None,
            }
        }
    }

    impl SendDdcBus for Bus {
        type Error = DdcErrorKind;

        fn capabilities(&self) -> DdcCapabilities {
            self.caps
        }

        async fn write(&self, _addr: u8, bytes: &[u8]) -> Result<(), DdcErrorKind> {
            if let Some(kind) = self.fail {
                return Err(kind);
            }
            self.registers[usize::from(bytes[0])].store(bytes[1], Ordering::Relaxed);
            Ok(())
        }

        async fn write_read(
            &self,
            _addr: u8,
            bytes: &[u8],
            buf: &mut [u8],
        ) -> Result<(), DdcErrorKind> {
            if let Some(kind) = self.fail {
                return Err(kind);
            }
            if buf.len() > usize::from(self.caps.write_read_max().1) {
                return Err(DdcErrorKind::Unsupported);
            }
            self.write_reads.fetch_add(1, Ordering::Relaxed);
            for (i, b) in buf.iter_mut().enumerate() {
                let reg = usize::from(bytes[0].wrapping_add(i as u8));
                *b = self.registers[reg].load(Ordering::Relaxed);
            }
            Ok(())
        }

        async fn read(&self, _addr: u8, _buf: &mut [u8]) -> Result<(), DdcErrorKind> {
            Err(DdcErrorKind::Unsupported)
        }

        async fn read_segmented(
            &self,
            _: EddcTarget,
            _: u8,
            _: u8,
            _: &mut [u8],
        ) -> Result<(), DdcErrorKind> {
            Err(DdcErrorKind::Unsupported)
        }
    }

    /// A client that requires `SendScdcTransport` of its transport, as a client generic over
    /// the `Send` twins does.
    struct SendClient<T>(T);

    impl<T: SendScdcTransport> SendClient<T> {
        fn update_flags(&self) -> impl Future<Output = Result<u8, T::Error>> + Send + '_ {
            SendScdcTransport::read(&self.0, 0x10)
        }
    }

    #[pollster::test]
    async fn plain_adapter_reads_and_writes_at_0x54() {
        let bus = Bus::scdc(32);
        let mut scdc = ScdcOverDdc::new(&bus).ok().unwrap();
        assert!(!scdc.is_bytewise());
        scdc.write(0x20, 0x03).await.unwrap();
        assert_eq!(scdc.read(0x20).await, Ok(0x03));
        let mut ced = [0u8; 7];
        scdc.read_block(0x50, &mut ced).await.unwrap();
        assert_eq!(ced, [0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56]);
        assert_eq!(bus.write_reads.load(Ordering::Relaxed), 2);
        assert!(core::ptr::eq(scdc.into_inner(), &bus));
    }

    #[pollster::test]
    async fn plain_adapter_bytewise_and_errors() {
        let bus = Bus::scdc(1);
        assert!(ScdcOverDdc::new(&bus).is_err());
        let scdc = ScdcOverDdc::bytewise(&bus).ok().unwrap();
        assert!(scdc.is_bytewise());
        let mut buf = [0u8; 3];
        scdc.read_block(0x40, &mut buf).await.unwrap();
        assert_eq!(buf, [0x40, 0x41, 0x42]);
        assert_eq!(bus.write_reads.load(Ordering::Relaxed), 3);

        let mut failing = Bus::scdc(32);
        failing.fail = Some(DdcErrorKind::Busy);
        let mut scdc = ScdcOverDdc::new(&failing).ok().unwrap();
        assert_eq!(scdc.read(0x21).await, Err(DdcErrorKind::Busy));
        assert_eq!(scdc.write(0x30, 0x01).await, Err(DdcErrorKind::Busy));
    }

    #[pollster::test]
    async fn plain_adapter_leaves_reads_beyond_the_bus_limit_to_the_bus() {
        let bus = Bus::scdc(16);
        let scdc = ScdcOverDdc::new(&bus).ok().unwrap();
        let mut long = [0u8; 17];
        assert_eq!(
            scdc.read_block(0x00, &mut long).await,
            Err(DdcErrorKind::Unsupported)
        );
    }

    #[pollster::test]
    async fn adapters_refuse_a_bus_that_cannot_serve_scdc() {
        let no_scdc = Arc::new(Bus::with(
            DdcCapabilities::general().with_address_reachable(SCDC, false),
        ));
        assert!(ScdcOverDdc::new(&*no_scdc).is_err());
        assert!(SendScdcOverDdc::new(Arc::clone(&no_scdc)).is_err());
        assert!(SendScdcOverDdc::bytewise(Arc::clone(&no_scdc)).is_err());
        let short_reads = Arc::new(Bus::scdc(15));
        assert!(SendScdcOverDdc::new(Arc::clone(&short_reads)).is_err());
        assert!(SendScdcOverDdc::bytewise(short_reads).is_ok());
        let no_reads = Bus::scdc(0);
        assert!(ScdcOverDdc::bytewise(&no_reads).is_err());
        let mut buf = [0u8; 1];
        assert_eq!(
            SendDdcBus::read(&no_reads, SCDC, &mut buf).await,
            Err(DdcErrorKind::Unsupported)
        );
        assert_eq!(
            SendDdcBus::read_segmented(&no_reads, EddcTarget::Edid, 0, 0, &mut buf).await,
            Err(DdcErrorKind::Unsupported)
        );
        assert_eq!(Fail(0).kind(), DdcErrorKind::Other);
    }

    /// Generic over every `SendDdcBus`: the client bounded by `SendScdcTransport` takes the
    /// `Send` adapter over an `Arc`, and its future is `Send`.
    fn spawn_client<B: SendDdcBus + 'static>(
        bus: Arc<B>,
    ) -> impl Future<Output = Result<u8, B::Error>> + Send {
        let client = SendClient(SendScdcOverDdc::new(bus).ok().expect("serves SCDC"));
        async move { client.update_flags().await }
    }

    #[pollster::test]
    async fn send_adapter_serves_a_client_bounded_by_the_send_twin() {
        let bus = Arc::new(Bus::scdc(32));
        assert_eq!(assert_send(spawn_client(Arc::clone(&bus))).await, Ok(0x10));

        let mut scdc = SendScdcOverDdc::bytewise(Arc::clone(&bus)).ok().unwrap();
        assert!(scdc.is_bytewise());
        SendScdcTransport::write(&mut scdc, 0x20, 0x07)
            .await
            .unwrap();
        let mut buf = [0u8; 2];
        SendScdcTransport::read_block(&scdc, 0x20, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, [0x07, 0x21]);
        let whole = SendScdcOverDdc::new(Arc::clone(&bus)).ok().unwrap();
        assert!(!whole.is_bytewise());
        SendScdcTransport::read_block(&whole, 0x20, &mut buf)
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&scdc.into_inner(), &bus));
        assert!(Arc::ptr_eq(&whole.into_inner(), &bus));
    }

    /// A firmware-style bus that is not `Send`.
    struct LocalBus(Cell<u8>, DdcCapabilities);

    impl DdcBus for LocalBus {
        type Error = ();
        fn capabilities(&self) -> DdcCapabilities {
            self.1
        }
        async fn write(&self, _addr: u8, bytes: &[u8]) -> Result<(), ()> {
            self.0.set(bytes[1]);
            Ok(())
        }
        async fn write_read(&self, _addr: u8, _bytes: &[u8], buf: &mut [u8]) -> Result<(), ()> {
            buf.fill(self.0.get());
            Ok(())
        }
        async fn read(&self, _addr: u8, _buf: &mut [u8]) -> Result<(), ()> {
            Err(())
        }
        async fn read_segmented(
            &self,
            _: EddcTarget,
            _: u8,
            _: u8,
            _: &mut [u8],
        ) -> Result<(), ()> {
            Err(())
        }
    }

    #[pollster::test]
    async fn plain_adapter_shares_a_bus_that_is_not_send() {
        let serves = DdcCapabilities::none()
            .with_address_reachable(SCDC, true)
            .with_write_max(2)
            .with_write_read_max(1, 16);
        let refused = Rc::new(LocalBus(Cell::new(0), DdcCapabilities::none()));
        assert!(ScdcOverDdc::new(refused).is_err());
        let bus = Rc::new(LocalBus(Cell::new(0), serves));
        let mut scdc = ScdcOverDdc::new(Rc::clone(&bus)).ok().unwrap();
        scdc.write(0x20, 0x05).await.unwrap();
        assert_eq!(scdc.read(0x20).await, Ok(0x05));
        let mut buf = [0u8; 1];
        assert_eq!(bus.read(SCDC, &mut buf).await, Err(()));
        assert_eq!(
            bus.read_segmented(EddcTarget::Edid, 0, 0, &mut buf).await,
            Err(())
        );
    }
}
