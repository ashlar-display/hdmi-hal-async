use display_types::cea861::hdmi_forum::HdmiForumFrl;
/// The PHY value types, at the paths `hdmi-hal` gives them (`hdmi_hal::phy::*`). They are
/// re-exported at the crate root as well.
pub use hdmi_hal::phy::{
    EqParams, FrlOutput, LaneEqParams, LanePatterns, LtpPattern, TxFfeLevel, TxFfeRestrictions,
};

/// Async link-level control of an HDMI 2.1 transmitter PHY.
///
/// This is the async counterpart to `hdmi_hal::phy::HdmiPhy`. The trait contract is
/// identical; only the method signatures gain `async`.
///
/// Covers what FRL link training needs from the transmitter: FRL rate selection, link
/// training patterns, FRL output mode, scrambling, and each lane's TxFFE level (FRL's
/// transmitter equalization, a combination of pre-shoot and de-emphasis) through
/// [`adjust_equalization`](Self::adjust_equalization). Further PHY configuration (lane
/// mapping, TMDS pre-emphasis and swing, spread-spectrum clocking and power states) is not
/// covered yet; it is planned for when the mode-setting layer needs it. Vendor-specific
/// register sequences are an implementation detail of each backend.
pub trait HdmiPhy {
    /// Error type returned by PHY operations.
    type Error;

    /// Select the FRL rate (or TMDS). Triggers the required lane reconfiguration sequence.
    ///
    /// Returns once the PHY transmits at `rate`, after any bring-up its hardware needs to
    /// get there — the Xilinx HDMI 2.1 transmitter, for example, holds a Nyquist clock
    /// pattern until its link is up. Only the PHY knows when that is, so the sequence
    /// belongs here; the link training layer sends no pattern of its own around it.
    async fn set_frl_rate(&mut self, rate: HdmiForumFrl) -> Result<(), Self::Error>;

    /// Drive the given link training patterns on the physical lanes, one per lane.
    async fn send_ltp(&mut self, patterns: LanePatterns) -> Result<(), Self::Error>;

    /// Select what the transmitter sends on the FRL lanes: gap characters only, or video,
    /// data islands and control periods.
    async fn set_frl_output(&mut self, output: FrlOutput) -> Result<(), Self::Error>;

    /// Adjust equalization parameters after link training feedback.
    async fn adjust_equalization(&mut self, params: EqParams) -> Result<(), Self::Error>;

    /// Enable or disable scrambling on the PHY.
    async fn set_scrambling(&mut self, enabled: bool) -> Result<(), Self::Error>;
}

/// [`HdmiPhy`] with `Send` futures, for code that spawns link training on a multi-threaded
/// executor.
///
/// `HdmiPhy`'s `async fn`s promise nothing about their futures, so code generic over
/// `P: HdmiPhy` cannot spawn a future that calls them, however `P` is bounded. This trait
/// has the same methods and contract, each returning a future that is `Send`, and errors
/// that are `Send` too: a spawned task returns them. Implementations still write
/// `async fn`; the compiler checks that each future is `Send`.
///
/// Every `SendHdmiPhy` is an `HdmiPhy`, through a blanket implementation, so it works
/// with everything written against `HdmiPhy` as well; a type implements one of the two
/// (implementing both conflicts with the blanket implementation). A `SendHdmiPhy` type
/// has both traits' methods, so with both traits in scope a method call on it is
/// ambiguous: import only the trait the code is bounded by, or call
/// `SendHdmiPhy::set_frl_rate(…)`.
///
/// Bound generic code by `SendHdmiPhy` alone. Adding `+ HdmiPhy`, which it implies
/// anyway, makes the compiler use that bound instead of the blanket implementation, and
/// the futures are no longer known to be `Send`.
pub trait SendHdmiPhy: Send {
    /// Error type returned by PHY operations.
    type Error: Send;

    /// See [`HdmiPhy::set_frl_rate`].
    fn set_frl_rate(
        &mut self,
        rate: HdmiForumFrl,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// See [`HdmiPhy::send_ltp`].
    fn send_ltp(
        &mut self,
        patterns: LanePatterns,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// See [`HdmiPhy::set_frl_output`].
    fn set_frl_output(
        &mut self,
        output: FrlOutput,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// See [`HdmiPhy::adjust_equalization`].
    fn adjust_equalization(
        &mut self,
        params: EqParams,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// See [`HdmiPhy::set_scrambling`].
    fn set_scrambling(
        &mut self,
        enabled: bool,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

impl<T: SendHdmiPhy> HdmiPhy for T {
    type Error = <T as SendHdmiPhy>::Error;

    fn set_frl_rate(
        &mut self,
        rate: HdmiForumFrl,
    ) -> impl Future<Output = Result<(), Self::Error>> {
        <T as SendHdmiPhy>::set_frl_rate(self, rate)
    }

    fn send_ltp(
        &mut self,
        patterns: LanePatterns,
    ) -> impl Future<Output = Result<(), Self::Error>> {
        <T as SendHdmiPhy>::send_ltp(self, patterns)
    }

    fn set_frl_output(
        &mut self,
        output: FrlOutput,
    ) -> impl Future<Output = Result<(), Self::Error>> {
        <T as SendHdmiPhy>::set_frl_output(self, output)
    }

    fn adjust_equalization(
        &mut self,
        params: EqParams,
    ) -> impl Future<Output = Result<(), Self::Error>> {
        <T as SendHdmiPhy>::adjust_equalization(self, params)
    }

    fn set_scrambling(&mut self, enabled: bool) -> impl Future<Output = Result<(), Self::Error>> {
        <T as SendHdmiPhy>::set_scrambling(self, enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal `HdmiPhy` that records what it is given. The tests that drive it method by
    /// method check little beyond recording — they show that the trait can be implemented
    /// and used for each method, and they keep this implementation covered. Behaviour is
    /// tested where it lives: in plumbob, against its simulated PHY.
    struct MockPhy {
        frl_rate: Option<HdmiForumFrl>,
        scrambling: Option<bool>,
        eq_calls: u32,
        last_eq: Option<EqParams>,
        frl_output: Option<FrlOutput>,
        last_ltp: Option<LanePatterns>,
    }

    impl MockPhy {
        fn new() -> Self {
            Self {
                frl_rate: None,
                scrambling: None,
                eq_calls: 0,
                last_eq: None,
                frl_output: None,
                last_ltp: None,
            }
        }
    }

    impl HdmiPhy for MockPhy {
        type Error = core::convert::Infallible;

        async fn set_frl_rate(&mut self, rate: HdmiForumFrl) -> Result<(), Self::Error> {
            self.frl_rate = Some(rate);
            Ok(())
        }

        async fn send_ltp(&mut self, patterns: LanePatterns) -> Result<(), Self::Error> {
            self.last_ltp = Some(patterns);
            Ok(())
        }

        async fn set_frl_output(&mut self, output: FrlOutput) -> Result<(), Self::Error> {
            self.frl_output = Some(output);
            Ok(())
        }

        async fn adjust_equalization(&mut self, params: EqParams) -> Result<(), Self::Error> {
            self.eq_calls += 1;
            self.last_eq = Some(params);
            Ok(())
        }

        async fn set_scrambling(&mut self, enabled: bool) -> Result<(), Self::Error> {
            self.scrambling = Some(enabled);
            Ok(())
        }
    }

    #[pollster::test]
    async fn send_ltp_records_patterns() {
        let mut phy = MockPhy::new();
        let patterns = LanePatterns {
            lane0: Some(LtpPattern::Lfsr0),
            lane1: Some(LtpPattern::Lfsr1),
            lane2: Some(LtpPattern::Lfsr2),
            lane3: None,
        };
        phy.send_ltp(patterns).await.unwrap();
        assert_eq!(phy.last_ltp, Some(patterns));
    }

    #[pollster::test]
    async fn send_ltp_updates_on_each_call() {
        let mut phy = MockPhy::new();
        let first = LanePatterns {
            lane0: Some(LtpPattern::Lfsr0),
            ..LanePatterns::default()
        };
        let second = LanePatterns {
            lane0: Some(LtpPattern::NyquistClock),
            ..first
        };
        phy.send_ltp(first).await.unwrap();
        phy.send_ltp(second).await.unwrap();
        assert_eq!(phy.last_ltp, Some(second));
    }

    #[pollster::test]
    async fn set_frl_rate_records_rate() {
        let mut phy = MockPhy::new();
        phy.set_frl_rate(HdmiForumFrl::Rate6Gbps4Lanes)
            .await
            .unwrap();
        assert_eq!(phy.frl_rate, Some(HdmiForumFrl::Rate6Gbps4Lanes));
    }

    #[pollster::test]
    async fn set_frl_rate_not_supported_is_valid() {
        let mut phy = MockPhy::new();
        phy.set_frl_rate(HdmiForumFrl::NotSupported).await.unwrap();
        assert_eq!(phy.frl_rate, Some(HdmiForumFrl::NotSupported));
    }

    #[pollster::test]
    async fn set_frl_rate_can_be_updated() {
        let mut phy = MockPhy::new();
        phy.set_frl_rate(HdmiForumFrl::Rate3Gbps3Lanes)
            .await
            .unwrap();
        phy.set_frl_rate(HdmiForumFrl::Rate12Gbps4Lanes)
            .await
            .unwrap();
        assert_eq!(phy.frl_rate, Some(HdmiForumFrl::Rate12Gbps4Lanes));
    }

    #[pollster::test]
    async fn set_scrambling_enable() {
        let mut phy = MockPhy::new();
        phy.set_scrambling(true).await.unwrap();
        assert_eq!(phy.scrambling, Some(true));
    }

    #[pollster::test]
    async fn set_scrambling_disable() {
        let mut phy = MockPhy::new();
        phy.set_scrambling(false).await.unwrap();
        assert_eq!(phy.scrambling, Some(false));
    }

    #[pollster::test]
    async fn set_scrambling_can_be_toggled() {
        let mut phy = MockPhy::new();
        phy.set_scrambling(true).await.unwrap();
        phy.set_scrambling(false).await.unwrap();
        assert_eq!(phy.scrambling, Some(false));
    }

    #[pollster::test]
    async fn set_frl_output_records_mode() {
        let mut phy = MockPhy::new();
        phy.set_frl_output(FrlOutput::GapOnly).await.unwrap();
        assert_eq!(phy.frl_output, Some(FrlOutput::GapOnly));
    }

    #[pollster::test]
    async fn set_frl_output_can_be_switched() {
        let mut phy = MockPhy::new();
        phy.set_frl_output(FrlOutput::GapOnly).await.unwrap();
        phy.set_frl_output(FrlOutput::Active).await.unwrap();
        assert_eq!(phy.frl_output, Some(FrlOutput::Active));
    }

    #[pollster::test]
    async fn adjust_equalization_records_per_lane_levels() {
        let mut phy = MockPhy::new();
        let mut params = EqParams::new();
        params.lane1.tx_ffe_level = TxFfeLevel::MAX;
        phy.adjust_equalization(params).await.unwrap();
        let last = phy.last_eq.unwrap();
        assert_eq!(last, params);
        assert_eq!(last.lane0.tx_ffe_level.value(), 0);
        assert_eq!(last.lane1.tx_ffe_level.value(), 7);
    }

    #[pollster::test]
    async fn adjust_equalization_tracks_call_count() {
        let mut phy = MockPhy::new();
        phy.adjust_equalization(EqParams::new()).await.unwrap();
        phy.adjust_equalization(EqParams::new()).await.unwrap();
        assert_eq!(phy.eq_calls, 2);
    }

    /// Implements only the `Send` twin; it records the calls in order.
    #[derive(Default)]
    struct SendMockPhy {
        calls: [Option<&'static str>; 5],
        n: usize,
        last_eq: Option<EqParams>,
    }

    impl SendMockPhy {
        fn record(&mut self, call: &'static str) {
            self.calls[self.n] = Some(call);
            self.n += 1;
        }
    }

    impl SendHdmiPhy for SendMockPhy {
        type Error = core::convert::Infallible;

        async fn set_frl_rate(&mut self, _rate: HdmiForumFrl) -> Result<(), Self::Error> {
            self.record("set_frl_rate");
            Ok(())
        }

        async fn send_ltp(&mut self, _patterns: LanePatterns) -> Result<(), Self::Error> {
            self.record("send_ltp");
            Ok(())
        }

        async fn set_frl_output(&mut self, _output: FrlOutput) -> Result<(), Self::Error> {
            self.record("set_frl_output");
            Ok(())
        }

        async fn adjust_equalization(&mut self, params: EqParams) -> Result<(), Self::Error> {
            self.record("adjust_equalization");
            self.last_eq = Some(params);
            Ok(())
        }

        async fn set_scrambling(&mut self, _enabled: bool) -> Result<(), Self::Error> {
            self.record("set_scrambling");
            Ok(())
        }
    }

    /// The result of each method call, in call order.
    type Results<E> = [Result<(), E>; 5];

    /// Calls every method through `HdmiPhy`, as code written against the plain trait does.
    async fn drive_plain<P: HdmiPhy>(phy: &mut P, eq: EqParams) -> Results<P::Error> {
        [
            phy.set_frl_rate(HdmiForumFrl::Rate6Gbps4Lanes).await,
            phy.send_ltp(LanePatterns::default()).await,
            phy.set_frl_output(FrlOutput::GapOnly).await,
            phy.adjust_equalization(eq).await,
            phy.set_scrambling(true).await,
        ]
    }

    /// Calls every method through `SendHdmiPhy`, as code bounded by the twin does.
    async fn drive_send<P: SendHdmiPhy>(phy: &mut P) -> Results<P::Error> {
        [
            SendHdmiPhy::set_frl_rate(phy, HdmiForumFrl::Rate12Gbps4Lanes).await,
            SendHdmiPhy::send_ltp(phy, LanePatterns::default()).await,
            SendHdmiPhy::set_frl_output(phy, FrlOutput::Active).await,
            SendHdmiPhy::adjust_equalization(phy, EqParams::new()).await,
            SendHdmiPhy::set_scrambling(phy, false).await,
        ]
    }

    /// The compiler proves this `Send` bound for every `P: SendHdmiPhy`, not only for the
    /// mock; with `HdmiPhy` in its place it would not compile.
    fn spawnable<P: SendHdmiPhy>(
        phy: &mut P,
    ) -> impl Future<Output = Results<P::Error>> + Send + '_ {
        drive_send(phy)
    }

    fn assert_send<T: Send>(value: T) -> T {
        value
    }

    const ALL_CALLS: [Option<&str>; 5] = [
        Some("set_frl_rate"),
        Some("send_ltp"),
        Some("set_frl_output"),
        Some("adjust_equalization"),
        Some("set_scrambling"),
    ];

    #[pollster::test]
    async fn send_twin_is_usable_as_hdmi_phy() {
        let mut phy = SendMockPhy::default();
        let mut eq = EqParams::new();
        eq.lane2.tx_ffe_level = TxFfeLevel::MAX;
        assert_eq!(drive_plain(&mut phy, eq).await, [Ok(()); 5]);
        assert_eq!(phy.calls, ALL_CALLS);
        assert_eq!(phy.last_eq, Some(eq));
    }

    #[pollster::test]
    async fn send_twin_futures_are_send_in_generic_code() {
        let mut phy = SendMockPhy::default();
        assert_eq!(assert_send(spawnable(&mut phy)).await, [Ok(()); 5]);
        assert_eq!(phy.calls, ALL_CALLS);
    }

    #[test]
    fn value_types_are_hdmi_hals_at_both_paths() {
        // `phy::` mirrors `hdmi_hal::phy::`; the crate root has the same types.
        let pattern: crate::phy::LtpPattern = hdmi_hal::phy::LtpPattern::Lfsr0;
        let _: crate::LtpPattern = pattern;
        let output: crate::phy::FrlOutput = hdmi_hal::phy::FrlOutput::GapOnly;
        let _: crate::FrlOutput = output;
        let level: crate::phy::TxFfeLevel = crate::TxFfeLevel::MAX;
        assert_eq!(level.value(), 7);
        let _: crate::phy::LanePatterns = crate::LanePatterns::default();
        let _: crate::phy::EqParams = crate::EqParams::default();
        let _: crate::phy::LaneEqParams = crate::LaneEqParams::default();
        let restrictions: crate::phy::TxFfeRestrictions =
            hdmi_hal::phy::TxFfeRestrictions::default();
        let _: crate::TxFfeRestrictions = restrictions;
    }
}
