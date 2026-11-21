//! Time for the procedures that drive the PHY and SCDC: a monotonic clock, an async delay,
//! and the [`Timestamp`] the clock reads.
//!
//! This is the async counterpart to `hdmi_hal::time`. [`Timestamp`] and [`Clock`] are
//! hdmi-hal's, re-exported: reading a counter does not wait, so the clock is neither `async`
//! nor fallible in either crate. [`Delay`] is async, and has a `Send` twin, [`SendDelay`].
//!
//! `Clock` has no twin, since `now` is not `async`, but code that holds a clock across an
//! `.await` in a task spawned on a multi-threaded executor bounds it `C: Clock + Sync`: a
//! future holding `&C` is `Send` only when `C` is `Sync`. A clock with state then keeps it in
//! an atomic or a lock, not a `Cell`.
//!
//! Two traits, not one: the clock is read where nothing waits (stamping an event, checking a
//! deadline), the delay is `async` while the clock is not, and a platform may take them from
//! different peripherals, a free-running counter and a timer. A procedure that needs both
//! bounds on both; a test double may implement both on one type, advancing its clock by each
//! delay.
//!
//! Neither trait has a forwarding implementation for references, as
//! [`DdcBus`](crate::ddc::DdcBus) has none: `impl<D: Delay> Delay for &mut D` would conflict
//! with [`SendDelay`]'s blanket implementation (E0119), since another crate may implement
//! `SendDelay` for a reference to its own type, and hdmi-hal's traits keep the same shape. An
//! implementation may implement either trait for a reference to its own type.
//!
//! That is also how one test double serves as both when a procedure takes the clock (`&C`)
//! and the delay (`&mut D`) as separate arguments: the double keeps its time in a `Cell` (or
//! an atomic, for `Send` code) and implements `Clock` for itself and `Delay` (or
//! `SendDelay`) for `&Double`, so `&double` and `&mut &double` can be passed at once.
//!
//! # Over embedded-hal-async
//!
//! [`Delay`] is the stack's own trait rather than a bound on embedded-hal-async's `DelayNs`:
//! generic code bounded by `DelayNs` cannot produce a `Send` future, and that crate has no
//! `Send` twin. An implementation over a `DelayNs` is a few lines: convert the duration to
//! microseconds, rounding up, and split values beyond `u32`.
//!
//! ```
//! use core::time::Duration;
//! use embedded_hal_async::delay::DelayNs;
//! use hdmi_hal_async::time::Delay;
//!
//! struct HalDelay<D>(D);
//!
//! impl<D: DelayNs> Delay for HalDelay<D> {
//!     async fn delay(&mut self, duration: Duration) {
//!         let mut us = duration.as_nanos().div_ceil(1_000);
//!         while us > 0 {
//!             let step = us.min(u128::from(u32::MAX));
//!             self.0.delay_us(step as u32).await;
//!             us -= step;
//!         }
//!     }
//! }
//! ```
//!
//! That covers the plain [`Delay`]. A [`SendDelay`] cannot be written generically over
//! `DelayNs`: its futures are not known to be `Send`, which is why the twin exists. A
//! `SendDelay` is implemented over a concrete timer whose future is `Send` (embassy-time's
//! timer, tokio's `sleep`), and a wrapper that implements `Delay` generically cannot also
//! implement `SendDelay` for one concrete timer (E0119).

use core::time::Duration;

pub use hdmi_hal::time::{Clock, Timestamp};

/// An async delay.
///
/// This is the async counterpart to `hdmi_hal::time::Delay`. The contract is identical; only
/// the method signature gains `async`.
///
/// Infallible: a timer that cannot wait leaves the caller nothing to recover with.
///
/// # Cancellation
///
/// The future may be dropped before it completes, for example when the caller races the
/// delay against an event. A dropped delay has done nothing that needs undoing, and the
/// implementation is ready for the next call.
pub trait Delay {
    /// Wait at least `duration`, rounding up to the implementation's own tick; never less,
    /// so a minimum wait holds whatever the timer.
    ///
    /// Measured on the platform's [`Clock`]: `now()` read after `delay(d)` completes is at
    /// least `d` later than `now()` read before it. A test double whose delay does not
    /// advance its clock breaks this.
    async fn delay(&mut self, duration: Duration);
}

/// [`Delay`] with a `Send` future, for procedures spawned on a multi-threaded executor.
///
/// The same method and contract, cancellation included, returning a future that is `Send`.
/// Every `SendDelay` is a `Delay`, through a blanket implementation; a type implements one of
/// the two. As with the other twins, bound generic code by `SendDelay` alone, and with both
/// traits in scope call `SendDelay::delay(…)` to avoid an ambiguous method call.
pub trait SendDelay: Send {
    /// See [`Delay::delay`].
    fn delay(&mut self, duration: Duration) -> impl Future<Output = ()> + Send;
}

impl<T: SendDelay> Delay for T {
    fn delay(&mut self, duration: Duration) -> impl Future<Output = ()> {
        <T as SendDelay>::delay(self, duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;

    /// A clock and delay on one type, the delay advancing the clock, as a test double does.
    struct SimTime {
        now: Cell<u64>,
    }

    impl Clock for SimTime {
        fn now(&self) -> Timestamp {
            Timestamp::from_micros(self.now.get())
        }
    }

    impl Delay for SimTime {
        async fn delay(&mut self, duration: Duration) {
            let us = duration.as_nanos().div_ceil(1_000) as u64;
            self.now.set(self.now.get() + us);
        }
    }

    async fn wait_until<T: Clock + Delay>(
        time: &mut T,
        deadline: Timestamp,
        poll: Duration,
    ) -> u32 {
        let mut polls = 0;
        while time.now() < deadline {
            time.delay(poll).await;
            polls += 1;
        }
        polls
    }

    #[pollster::test]
    async fn clock_and_delay_compose_on_one_type() {
        let mut time = SimTime { now: Cell::new(0) };
        let start = time.now();
        let deadline = start.checked_add(Duration::from_millis(5)).unwrap();
        assert_eq!(
            wait_until(&mut time, deadline, Duration::from_millis(1)).await,
            5
        );
        assert_eq!(
            time.now().checked_duration_since(start),
            Some(Duration::from_millis(5))
        );
    }

    /// Implements only the `Send` twin; it adds up what it was asked to wait.
    #[derive(Default)]
    struct SendMockDelay {
        waited: Duration,
    }

    impl SendDelay for SendMockDelay {
        async fn delay(&mut self, duration: Duration) {
            self.waited += duration;
        }
    }

    async fn wait_plain<D: Delay>(delay: &mut D) {
        delay.delay(Duration::from_millis(1)).await;
    }

    async fn wait_send<D: SendDelay>(delay: &mut D) {
        SendDelay::delay(delay, Duration::from_millis(2)).await;
    }

    /// The compiler proves this `Send` bound for every `D: SendDelay`.
    fn spawnable<D: SendDelay>(delay: &mut D) -> impl Future<Output = ()> + Send + '_ {
        wait_send(delay)
    }

    fn assert_send<T: Send>(value: T) -> T {
        value
    }

    #[pollster::test]
    async fn send_twin_is_usable_as_delay() {
        let mut delay = SendMockDelay::default();
        wait_plain(&mut delay).await;
        assert_eq!(delay.waited, Duration::from_millis(1));
    }

    #[pollster::test]
    async fn send_twin_future_is_send_in_generic_code() {
        let mut delay = SendMockDelay::default();
        assert_send(spawnable(&mut delay)).await;
        assert_eq!(delay.waited, Duration::from_millis(2));
    }

    #[test]
    fn timestamp_and_clock_are_hdmi_hals() {
        let t: Timestamp = hdmi_hal::time::Timestamp::from_micros(7);
        let _: hdmi_hal::time::Timestamp = t;
        fn takes_sync_clock<C: hdmi_hal::time::Clock>(clock: &C) -> Timestamp {
            clock.now()
        }
        let time = SimTime { now: Cell::new(3) };
        assert_eq!(takes_sync_clock(&time).as_micros(), 3);
    }
}
