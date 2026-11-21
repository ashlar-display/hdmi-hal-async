extern crate alloc;

use super::*;
use alloc::rc::Rc;
use core::cell::Cell;
use core::sync::atomic::{AtomicU32, Ordering};

/// A `Send` bus over an EDID store, counting transactions with an atomic.
struct SendBus {
    edid: [u8; 256],
    transactions: AtomicU32,
}

impl SendBus {
    fn new() -> Self {
        SendBus {
            edid: core::array::from_fn(|i| i as u8),
            transactions: AtomicU32::new(0),
        }
    }
}

impl SendDdcBus for SendBus {
    type Error = DdcErrorKind;

    fn capabilities(&self) -> DdcCapabilities {
        DdcCapabilities::none()
            .with_eddc_target(EddcTarget::Edid, true)
            .with_max_segment(Some(0))
    }

    async fn write(&self, _addr: u8, _bytes: &[u8]) -> Result<(), DdcErrorKind> {
        Err(DdcErrorKind::Unsupported)
    }

    async fn write_read(
        &self,
        _addr: u8,
        _bytes: &[u8],
        _buf: &mut [u8],
    ) -> Result<(), DdcErrorKind> {
        Err(DdcErrorKind::Unsupported)
    }

    async fn read(&self, _addr: u8, _buf: &mut [u8]) -> Result<(), DdcErrorKind> {
        Err(DdcErrorKind::Unsupported)
    }

    async fn read_segmented(
        &self,
        target: EddcTarget,
        segment: u8,
        offset: u8,
        buf: &mut [u8],
    ) -> Result<(), DdcErrorKind> {
        let start = usize::from(offset);
        if target != EddcTarget::Edid || segment != 0 || start + buf.len() > 256 {
            return Err(DdcErrorKind::Unsupported);
        }
        self.transactions.fetch_add(1, Ordering::Relaxed);
        buf.copy_from_slice(&self.edid[start..start + buf.len()]);
        Ok(())
    }
}

/// A firmware-style bus that is not `Send`: interior state in a `Cell`.
struct LocalBus {
    register: Cell<u8>,
    transactions: Cell<u32>,
}

impl DdcBus for LocalBus {
    type Error = ();

    fn capabilities(&self) -> DdcCapabilities {
        DdcCapabilities::none()
            .with_address_reachable(SCDC, true)
            .with_write_max(2)
            .with_write_read_max(1, 1)
    }

    async fn write(&self, _addr: u8, bytes: &[u8]) -> Result<(), ()> {
        self.transactions.set(self.transactions.get() + 1);
        self.register.set(bytes[1]);
        Ok(())
    }

    async fn write_read(&self, _addr: u8, _bytes: &[u8], buf: &mut [u8]) -> Result<(), ()> {
        self.transactions.set(self.transactions.get() + 1);
        buf[0] = self.register.get();
        Ok(())
    }

    async fn read(&self, _addr: u8, _buf: &mut [u8]) -> Result<(), ()> {
        Err(())
    }

    async fn read_segmented(&self, _: EddcTarget, _: u8, _: u8, _: &mut [u8]) -> Result<(), ()> {
        Err(())
    }
}

/// Reads one EDID block through the plain trait, as code written against `DdcBus` does.
async fn read_block_plain<B: DdcBus + ?Sized>(bus: &B, buf: &mut [u8]) -> Result<(), B::Error> {
    bus.read_segmented(EddcTarget::Edid, 0, 0x80, buf).await
}

/// The compiler proves this `Send` bound for every `B: SendDdcBus`; with `DdcBus` in its
/// place it would not compile.
fn spawnable<'a, B: SendDdcBus>(
    bus: &'a B,
    buf: &'a mut [u8],
) -> impl Future<Output = Result<(), B::Error>> + Send + 'a {
    SendDdcBus::read_segmented(bus, EddcTarget::Edid, 0, 0, buf)
}

fn assert_send<T: Send>(value: T) -> T {
    value
}

#[pollster::test]
async fn send_twin_futures_are_send_in_generic_code() {
    let bus = SendBus::new();
    let mut buf = [0u8; 4];
    assert_send(spawnable(&bus, &mut buf)).await.unwrap();
    assert_eq!(buf, [0, 1, 2, 3]);
}

#[pollster::test]
async fn send_twin_is_usable_as_ddc_bus() {
    let bus = SendBus::new();
    let mut buf = [0u8; 2];
    read_block_plain(&bus, &mut buf).await.unwrap();
    assert_eq!(buf, [0x80, 0x81]);
    assert!(DdcBus::capabilities(&bus).reads_target(EddcTarget::Edid));
    assert_eq!(
        DdcBus::write(&bus, SCDC, &[0x20, 0x01]).await,
        Err(DdcErrorKind::Unsupported)
    );
    assert_eq!(
        DdcBus::write_read(&bus, SCDC, &[0x20], &mut buf).await,
        Err(DdcErrorKind::Unsupported)
    );
    assert_eq!(
        DdcBus::read(&bus, DDC_CI, &mut buf).await,
        Err(DdcErrorKind::Unsupported)
    );
    assert_eq!(bus.transactions.load(Ordering::Relaxed), 1);
}

#[pollster::test]
async fn a_bus_that_is_not_send_is_shared_through_references_and_rc() {
    let bus = Rc::new(LocalBus {
        register: Cell::new(0),
        transactions: Cell::new(0),
    });
    let writer = Rc::clone(&bus);
    let reader: &LocalBus = &bus;

    writer.write(SCDC, &[0x20, 0x03]).await.unwrap();
    let mut value = [0u8; 1];
    reader.write_read(SCDC, &[0x20], &mut value).await.unwrap();
    assert_eq!(value, [0x03]);
    assert_eq!(bus.transactions.get(), 2);
    assert_eq!(reader.read(SCDC, &mut value).await, Err(()));
    assert_eq!(
        reader
            .read_segmented(EddcTarget::Edid, 0, 0, &mut value)
            .await,
        Err(())
    );
}

#[test]
fn the_vocabulary_is_hdmi_hals() {
    fn same<T>(_: T, _: T) {}
    same(DdcErrorKind::Timeout, hdmi_hal::ddc::DdcErrorKind::Timeout);
    same(
        DdcCapabilities::none(),
        hdmi_hal::ddc::DdcCapabilities::none(),
    );
    same(EddcTarget::Edid, hdmi_hal::ddc::EddcTarget::Edid);
    assert_eq!(EDID, hdmi_hal::ddc::EDID);
    assert!(is_usable_address(SCDC));
}
