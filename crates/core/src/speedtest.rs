//! Speed test state: the phases and readings the runner feeds in. `build.rs`
//! lays them out like Omarchy's own speed test (twin Download / Upload
//! gauges on a shared scale) using the same cards as the other panels.

/// Full-scale latch points shared by both gauges, smallest first (Omarchy's
/// `SpeedTestOverlay.scaleStops`). A fresh run starts from the first one.
const SCALE_STOPS: [f32; 7] = [100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0];
/// How long the opening sweep (gauges to full scale and back) lasts.
const IGNITION_DURATION: f32 = 0.95;
/// When, within the sweep, the gauges head for full scale. Waiting one frame
/// lets the gauges appear empty first so the rise animates.
const SWEEP_RISE_AT: f32 = 0.06;
/// When, within the sweep, the gauges fall back to zero.
const SWEEP_FALL_AT: f32 = 0.55;

#[derive(Clone, Debug, PartialEq)]
pub enum SpeedPhase {
    Ignition,
    Download,
    Upload,
    Complete,
    Error(String),
}

/// Live events pushed from the speedtest runner into the scene.
#[derive(Clone, Debug, PartialEq)]
pub enum SpeedTestEvent {
    Ping(f32),
    DownloadSample(f32),
    DownloadDone(f32),
    UploadStart,
    UploadSample(f32),
    UploadDone(f32),
    Error(String),
}

#[derive(Clone, Debug)]
pub struct SpeedTestState {
    /// Name of the connection under test.
    pub connection: String,
    pub phase: SpeedPhase,
    pub ping_ms: Option<f32>,
    pub download_mbps: Option<f32>,
    pub upload_mbps: Option<f32>,
    /// Full scale of both gauges, in Mbps.
    pub scale: f32,
    phase_elapsed: f32,
}

impl SpeedTestState {
    pub fn new(connection: String) -> Self {
        Self {
            connection,
            phase: SpeedPhase::Ignition,
            ping_ms: None,
            download_mbps: None,
            upload_mbps: None,
            scale: SCALE_STOPS[0],
            phase_elapsed: 0.0,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(
            self.phase,
            SpeedPhase::Ignition | SpeedPhase::Download | SpeedPhase::Upload
        )
    }

    /// Whether the active phase is still waiting for its first reading.
    pub fn waiting(&self) -> bool {
        match self.phase {
            SpeedPhase::Download => self.download_mbps.is_none(),
            SpeedPhase::Upload => self.upload_mbps.is_none(),
            _ => false,
        }
    }

    /// During the opening sweep, where both gauges are headed (0 or 1).
    pub fn sweep(&self) -> Option<f32> {
        if self.phase != SpeedPhase::Ignition {
            return None;
        }
        let up = (SWEEP_RISE_AT..SWEEP_FALL_AT).contains(&self.phase_elapsed);
        Some(if up { 1.0 } else { 0.0 })
    }

    pub fn apply(&mut self, event: SpeedTestEvent) {
        match event {
            SpeedTestEvent::Ping(ms) => {
                if ms.is_finite() && ms >= 0.0 {
                    self.ping_ms = Some(ms);
                }
            }
            SpeedTestEvent::DownloadSample(mbps) => {
                if !mbps.is_finite() || mbps < 0.0 {
                    return;
                }
                if self.phase == SpeedPhase::Ignition {
                    self.enter(SpeedPhase::Download);
                }
                if self.phase == SpeedPhase::Download {
                    self.download_mbps = Some(mbps);
                    self.expand_scale(mbps);
                }
            }
            SpeedTestEvent::DownloadDone(mbps) => {
                if mbps.is_finite() && mbps > 0.0 {
                    self.download_mbps = Some(mbps);
                    self.expand_scale(mbps);
                }
            }
            SpeedTestEvent::UploadStart => {
                if self.is_running() {
                    self.enter(SpeedPhase::Upload);
                }
            }
            SpeedTestEvent::UploadSample(mbps) => {
                if !mbps.is_finite() || mbps < 0.0 || !self.is_running() {
                    return;
                }
                if self.phase != SpeedPhase::Upload {
                    self.enter(SpeedPhase::Upload);
                }
                self.upload_mbps = Some(mbps);
                self.expand_scale(mbps);
            }
            SpeedTestEvent::UploadDone(mbps) => {
                if mbps.is_finite() && mbps > 0.0 {
                    self.upload_mbps = Some(mbps);
                    self.expand_scale(mbps);
                }
                self.enter(SpeedPhase::Complete);
            }
            SpeedTestEvent::Error(msg) => self.enter(SpeedPhase::Error(msg)),
        }
    }

    fn enter(&mut self, phase: SpeedPhase) {
        self.phase = phase;
        self.phase_elapsed = 0.0;
    }

    /// Latches the shared scale up (never down) so `value` fits.
    fn expand_scale(&mut self, value: f32) {
        let fit = SCALE_STOPS
            .iter()
            .copied()
            .find(|&stop| value <= stop * 0.92)
            .unwrap_or(SCALE_STOPS[SCALE_STOPS.len() - 1]);
        self.scale = self.scale.max(fit);
    }

    /// Advances the opening sweep. Returns true when the layout changes.
    pub fn step(&mut self, dt: f32) -> bool {
        let before = (self.phase.clone(), self.sweep());
        self.phase_elapsed += dt.clamp(0.0, 0.05);
        if self.phase == SpeedPhase::Ignition && self.phase_elapsed >= IGNITION_DURATION {
            self.enter(SpeedPhase::Download);
        }
        before != (self.phase.clone(), self.sweep())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_latches_up_and_shares_between_phases() {
        let mut st = SpeedTestState::new("Wi-Fi".into());
        st.apply(SpeedTestEvent::DownloadSample(420.0));
        assert_eq!(st.scale, 500.0);
        st.apply(SpeedTestEvent::DownloadSample(80.0));
        assert_eq!(st.scale, 500.0);
        st.apply(SpeedTestEvent::UploadStart);
        st.apply(SpeedTestEvent::UploadSample(940.0));
        assert_eq!(st.scale, 2500.0);
    }

    #[test]
    fn opening_sweep_rises_then_falls_into_download() {
        let mut st = SpeedTestState::new("Wi-Fi".into());
        assert_eq!(st.sweep(), Some(0.0));
        let mut changes = 0;
        for _ in 0..80 {
            changes += st.step(0.016) as usize;
        }
        assert_eq!(changes, 3, "rise, fall, then the download phase");
        assert_eq!(st.phase, SpeedPhase::Download);
        assert!(st.waiting());
    }

    #[test]
    fn samples_after_an_error_are_ignored() {
        let mut st = SpeedTestState::new("Wi-Fi".into());
        st.apply(SpeedTestEvent::Error("offline".into()));
        st.apply(SpeedTestEvent::UploadSample(12.0));
        assert_eq!(st.phase, SpeedPhase::Error("offline".into()));
        assert_eq!(st.upload_mbps, None);
    }
}
