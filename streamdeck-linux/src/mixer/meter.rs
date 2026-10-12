//! Measured peaks from a previously verified OWNED virtual sink monitor only.
//! PulseAudio's PEAK_DETECT selects peak resampling; it is not an RMS meter.
//! Sources: pulseaudio/src/pulse/def.h and pulsecore/resampler/peaks.c.
use super::*;
use pulse::{
    def::BufferAttr,
    proplist::Proplist,
    sample::{Format, Spec},
    stream::{FlagSet, PeekResult, State, Stream},
};

const RATE: u32 = 50;
const STALE: Duration = Duration::from_millis(250);
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub source: u32,
    pub module: u32,
    pub name: String,
}
pub(super) struct Meter {
    pub target: Target,
    stream: Stream,
    sample: Option<(Instant, f32)>,
}
impl Meter {
    /// The caller proves source is this engine's null-sink monitor. Numeric
    /// selection + DONT_MOVE forbids fallback to a microphone/default source.
    pub fn new(context: &mut Context, target: Target, key: &str) -> Result<Self> {
        let mut props = Proplist::new().context("Create meter properties")?;
        for (key, value) in [
            ("media.name", format!("Deckforge meter {key}")),
            ("media.role", "abstract".into()),
            ("deckforge.meter", "true".into()),
            ("deckforge.meter.source", target.name.clone()),
        ] {
            props
                .set_str(key, &value)
                .map_err(|_| anyhow::anyhow!("Invalid meter properties"))?;
        }
        let spec = Spec {
            format: Format::F32le,
            rate: RATE,
            channels: 2,
        };
        let mut stream = Stream::new_with_proplist(
            context,
            "Deckforge owned bus meter",
            &spec,
            None,
            &mut props,
        )
        .context("Create owned bus meter")?;
        let attributes = BufferAttr {
            maxlength: 4096,
            tlength: u32::MAX,
            prebuf: u32::MAX,
            minreq: u32::MAX,
            fragsize: 8,
        };
        stream.connect_record(
            Some(&target.source.to_string()),
            Some(&attributes),
            FlagSet::PEAK_DETECT
                | FlagSet::DONT_MOVE
                | FlagSet::DONT_INHIBIT_AUTO_SUSPEND
                | FlagSet::ADJUST_LATENCY
                | FlagSet::NO_REMIX_CHANNELS,
        )?;
        Ok(Self {
            target,
            stream,
            sample: None,
        })
    }
    pub fn failed(&self) -> bool {
        matches!(self.stream.get_state(), State::Failed | State::Terminated)
    }
    pub fn level(&mut self, now: Instant) -> Level {
        if self.stream.get_state() != State::Ready
            || self.stream.get_device_index() != Some(self.target.source)
        {
            self.sample = None;
            return Level::default();
        }
        let spec = self.stream.get_sample_spec();
        if !spec.is_some_and(|s| s.format == Format::F32le && s.channels == 2 && s.rate == RATE) {
            self.sample = None;
            return Level::default();
        }
        if self.stream.is_suspended() == Ok(true) {
            // This verified virtual monitor is known suspended/idle, so there
            // is no flowing audio. Do not keep it awake just to generate zeros.
            self.sample = None;
            return Level {
                peak: 0.0,
                available: true,
            };
        }
        let mut measured = None::<f32>;
        for _ in 0..64 {
            let sample = match self.stream.peek() {
                Ok(PeekResult::Empty) => break,
                Ok(PeekResult::Data(bytes)) => peaks(bytes),
                Ok(PeekResult::Hole(_)) => None,
                Err(_) => {
                    self.sample = None;
                    return Level::default();
                }
            };
            if self.stream.discard().is_err() {
                self.sample = None;
                return Level::default();
            }
            if let Some(value) = sample {
                measured = Some(measured.map_or(value, |old| old.max(value)));
            } else {
                self.sample = None;
            }
        }
        if let Some(value) = measured {
            self.sample = Some((now, value));
        }
        self.sample
            .filter(|(at, _)| now.saturating_duration_since(*at) <= STALE)
            .map(|(_, peak)| Level {
                peak,
                available: true,
            })
            .unwrap_or_default()
    }
}
impl Drop for Meter {
    fn drop(&mut self) {
        let _ = self.stream.disconnect();
    }
}
fn peaks(bytes: &[u8]) -> Option<f32> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(8) || bytes.len() > 65536 {
        return None;
    }
    let mut peak = 0.0f32;
    for sample in bytes.chunks_exact(4) {
        let value = f32::from_le_bytes(sample.try_into().unwrap());
        if !value.is_finite() {
            return None;
        }
        peak = peak.max(value.abs());
    }
    Some(peak)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn peaks_measure_both_channels_without_phase_cancellation() {
        let data = [0.25f32, -0.25, 0.125, 0.8, -1.2, 0.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
        assert_eq!(peaks(&data), Some(1.2));
        assert_eq!(peaks(&[0; 8]), Some(0.0));
    }
    #[test]
    fn malformed_nonfinite_and_unbounded_data_is_unavailable() {
        assert_eq!(peaks(&[]), None);
        assert_eq!(peaks(&[0; 7]), None);
        assert_eq!(peaks(&vec![0; 65544]), None);
        for sample in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let data = [sample, 0.0]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>();
            assert_eq!(peaks(&data), None);
        }
    }
}
