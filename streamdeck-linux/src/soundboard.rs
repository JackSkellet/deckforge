//! Streaming soundboard mixer. This module opens audio files but never opens an
//! audio device, starts a process/thread, or performs OS playback.
//!
//! Mode/fade numbers come from evidence/dial-system-contracts.json. Linear gain,
//! linear fade curves, voice limits and replacement policy are explicit Linux
//! implementation choices; native Windows playback timing was not observed.
//! Rodio exposes mid-stream decoder exhaustion as EOF; it does not give this
//! iterator a separate error value for every malformed packet.
use anyhow::{Context, Result, bail, ensure};
use rodio::{Decoder, Source, source::UniformSourceIterator};
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    fs::File,
    io::BufReader,
    num::NonZero,
    path::PathBuf,
};

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;
const MAX_VOICES: usize = 32;
const MAX_CONTEXT_VOICES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Overlap,
    PlayStop,
    Restart,
    LoopStop,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fade {
    None,
    In,
    Out,
    Both,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub path: PathBuf,
    /// Empty means the default output. Other values are caller-supplied Linux sink IDs.
    pub output: String,
    mode: Mode,
    fade: Fade,
    fade_frames: usize,
    gain: f64,
}
impl Config {
    pub fn parse(settings: &Value) -> Result<Self> {
        ensure!(
            settings.is_object(),
            "Soundboard settings must be an object"
        );
        let path = settings["path"]
            .as_str()
            .filter(|p| !p.trim().is_empty())
            .context("Choose an audio file")?;
        let mode = match enum_value(settings, "actionType", 1)? {
            0 => Mode::Overlap,
            1 => Mode::PlayStop,
            2 => Mode::Restart,
            3 => Mode::LoopStop,
            other => bail!("Unsupported soundboard actionType {other}"),
        };
        let fade = match enum_value(settings, "fadeType", 0)? {
            0 => Fade::None,
            1 => Fade::In,
            2 => Fade::Out,
            3 => Fade::Both,
            other => bail!("Unsupported soundboard fadeType {other}"),
        };
        let seconds = number(settings, "fadeLen", 1.0)?;
        ensure!(
            (1.0..=5.0).contains(&seconds),
            "Soundboard fadeLen must be between 1 and 5 seconds"
        );
        let volume = number(settings, "volume", 50.0)?;
        ensure!(
            (0.0..=100.0).contains(&volume),
            "Soundboard volume must be between 0 and 100"
        );
        let output = if let Some(output) = settings.get("linuxOutput") {
            output
                .as_str()
                .context("linuxOutput must be a Linux sink ID string")?
                .trim()
                .to_owned()
        } else {
            let foreign = match settings.get("outputType") {
                None => "",
                Some(value) => value.as_str().context("outputType must be a string")?,
            };
            ensure!(
                foreign.is_empty(),
                "Imported outputType has no verified Linux mapping; set linuxOutput explicitly"
            );
            String::new()
        };
        ensure!(output.len() <= 4096, "Linux output ID is too long");
        Ok(Self {
            path: PathBuf::from(path),
            output,
            mode,
            fade,
            fade_frames: (seconds * f64::from(SAMPLE_RATE)).round() as usize,
            gain: volume / 100.0,
        })
    }
    fn fade_in(&self) -> bool {
        matches!(self.fade, Fade::In | Fade::Both)
    }
    fn fade_out(&self) -> bool {
        matches!(self.fade, Fade::Out | Fade::Both)
    }
}
fn enum_value(settings: &Value, key: &str, default: u64) -> Result<u64> {
    match settings.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_u64()
            .with_context(|| format!("{key} must be a nonnegative integer")),
    }
}
fn number(settings: &Value, key: &str, default: f64) -> Result<f64> {
    match settings.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_f64()
            .filter(|v| v.is_finite())
            .with_context(|| format!("{key} must be a finite number")),
    }
}

struct StopFade {
    frames_left: usize,
    frames_total: usize,
    start_gain: f64,
}
struct Voice {
    context: String,
    config: Config,
    source: UniformSourceIterator<Box<dyn Source + Send>>,
    ahead: VecDeque<[f32; 2]>,
    capacity: usize,
    source_ended: bool,
    duration_frames: Option<u64>,
    age: u64,
    stopping: Option<StopFade>,
    finished: bool,
}
impl Voice {
    fn open(context: &str, config: Config) -> Result<Self> {
        ensure!(
            std::fs::metadata(&config.path)
                .with_context(|| format!("Inspect audio file {}", config.path.display()))?
                .is_file(),
            "Soundboard path must be a regular file"
        );
        let file = File::open(&config.path)
            .with_context(|| format!("Open audio file {}", config.path.display()))?;
        let length = file.metadata()?.len();
        let builder = Decoder::builder()
            .with_data(BufReader::new(file))
            .with_byte_len(length)
            .with_seekable(true)
            .with_gapless(true);
        // A looped decoder seeks/reinitializes on EOF; it does not cache a complete
        // decoded file as buffered().repeat_infinite() would.
        let source: Box<dyn Source + Send> = if config.mode == Mode::LoopStop {
            Box::new(
                builder
                    .build_looped()
                    .context("Decode looping audio file")?,
            )
        } else {
            Box::new(builder.build().context("Decode audio file")?)
        };
        ensure!(
            source.channels().get() <= 2,
            "Only mono and stereo audio are supported; multichannel downmix is not implemented"
        );
        ensure!(
            (8_000..=384_000).contains(&source.sample_rate().get()),
            "Unsupported audio sample rate"
        );
        let duration_frames = source.total_duration().and_then(|d| {
            let frames = (d.as_nanos() * u128::from(SAMPLE_RATE) + 500_000_000) / 1_000_000_000;
            u64::try_from(frames).ok().filter(|&frames| frames > 0)
        });
        let capacity = if config.fade_out() && config.mode != Mode::LoopStop {
            config.fade_frames + 1
        } else {
            2
        };
        let source = UniformSourceIterator::new(
            source,
            NonZero::new(CHANNELS).unwrap(),
            NonZero::new(SAMPLE_RATE).unwrap(),
        );
        let mut voice = Self {
            context: context.into(),
            config,
            source,
            ahead: VecDeque::with_capacity(capacity),
            capacity,
            source_ended: false,
            duration_frames,
            age: 0,
            stopping: None,
            finished: false,
        };
        voice.fill_ahead();
        ensure!(
            !voice.ahead.is_empty(),
            "Audio file contains no decodable samples"
        );
        Ok(voice)
    }
    fn fill_ahead(&mut self) {
        while !self.source_ended && self.ahead.len() < self.capacity {
            let Some(left) = self.source.next() else {
                self.source_ended = true;
                break;
            };
            let Some(right) = self.source.next() else {
                self.source_ended = true;
                break;
            };
            let clean = |sample: rodio::Sample| {
                let sample = f64::from(sample);
                if sample.is_finite() {
                    sample.clamp(f64::from(f32::MIN), f64::from(f32::MAX)) as f32
                } else {
                    0.0
                }
            };
            self.ahead.push_back([clean(left), clean(right)]);
        }
        if self.source_ended {
            // Actual decoded length overrides potentially inexact container metadata.
            self.duration_frames = Some(self.age.saturating_add(self.ahead.len() as u64));
        }
    }
    fn envelope(&self) -> f64 {
        if let Some(stop) = &self.stopping {
            return stop.start_gain * ramp_down(stop.frames_left, stop.frames_total);
        }
        let mut gain = 1.0;
        if self.config.fade_in() {
            let length = if self.config.mode == Mode::LoopStop {
                self.config.fade_frames as u64
            } else {
                self.duration_frames
                    .unwrap_or(u64::MAX)
                    .min(self.config.fade_frames as u64)
            };
            gain *= (self.age as f64 / length.saturating_sub(1).max(1) as f64).min(1.0);
        }
        if self.config.fade_out() && self.config.mode != Mode::LoopStop && self.source_ended {
            let total = self
                .duration_frames
                .unwrap_or(0)
                .min(self.config.fade_frames as u64)
                .max(1) as usize;
            gain *= ramp_down(self.ahead.len(), total);
        }
        gain
    }
    fn stop(&mut self) {
        if self.finished || self.stopping.is_some() {
            return;
        }
        if self.config.fade_out() {
            let start_gain = self.envelope();
            let frames = if self.source_ended && self.config.mode != Mode::LoopStop {
                self.config.fade_frames.min(self.ahead.len()).max(1)
            } else {
                self.config.fade_frames
            };
            self.stopping = Some(StopFade {
                frames_left: frames,
                frames_total: frames,
                start_gain,
            });
        } else {
            self.finished = true;
        }
    }
    fn next_frame(&mut self) -> Option<[f64; 2]> {
        if self.finished {
            return None;
        }
        self.fill_ahead();
        if self.ahead.is_empty() {
            self.finished = true;
            return None;
        }
        let gain = self.envelope() * self.config.gain;
        let [left, right] = self.ahead.pop_front().unwrap();
        self.age = self.age.saturating_add(1);
        if let Some(stop) = &mut self.stopping {
            stop.frames_left = stop.frames_left.saturating_sub(1);
            if stop.frames_left == 0 {
                self.finished = true;
            }
        }
        if self.source_ended && self.ahead.is_empty() {
            self.finished = true;
        }
        Some([f64::from(left) * gain, f64::from(right) * gain])
    }
}
fn ramp_down(left: usize, total: usize) -> f64 {
    if total <= 1 {
        0.0
    } else {
        (left.saturating_sub(1) as f64 / (total - 1) as f64).clamp(0.0, 1.0)
    }
}

#[derive(Default)]
pub struct Mixer {
    voices: Vec<Voice>,
}
impl Mixer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn trigger(&mut self, context: &str, config: Config) -> Result<()> {
        ensure!(
            !context.is_empty() && context.len() <= 4096,
            "Invalid soundboard action context"
        );
        self.voices.retain(|voice| !voice.finished);
        let current = self
            .voices
            .iter()
            .filter(|voice| voice.context == context)
            .count();
        if current > 0 && matches!(config.mode, Mode::PlayStop | Mode::LoopStop) {
            self.stop_context(context);
            return Ok(());
        }
        let retained = if config.mode == Mode::Restart {
            self.voices.len() - current
        } else {
            self.voices.len()
        };
        ensure!(
            retained < MAX_VOICES,
            "Soundboard voice limit reached ({MAX_VOICES})"
        );
        ensure!(
            config.mode == Mode::Restart || current < MAX_CONTEXT_VOICES,
            "Soundboard overlap limit reached ({MAX_CONTEXT_VOICES} for this action)"
        );
        let replacement = Voice::open(context, config)?;
        // Opening/priming a replacement must succeed before an existing voice is
        // removed. A bad path/format cannot silence a currently working action.
        if replacement.config.mode == Mode::Restart {
            self.voices.retain(|voice| voice.context != context);
        }
        self.voices.push(replacement);
        Ok(())
    }
    /// Explicit Stop Audio / hard shutdown: no fade, all files released immediately.
    pub fn stop_all(&mut self) {
        self.voices.clear();
    }
    /// Manual action stop honors each playing voice's configured fade-out.
    pub fn stop_context(&mut self, context: &str) {
        for voice in self
            .voices
            .iter_mut()
            .filter(|voice| voice.context == context)
        {
            voice.stop();
        }
        self.voices.retain(|voice| !voice.finished);
    }
    /// A failed output must not stop independent sinks, even for the same action.
    pub fn remove_output(&mut self, output: &str) {
        self.voices.retain(|voice| voice.config.output != output);
    }
    pub fn outputs(&self) -> Vec<String> {
        self.voices
            .iter()
            .filter(|voice| !voice.finished)
            .map(|voice| voice.config.output.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    pub fn active(&self) -> HashMap<String, usize> {
        let mut active = HashMap::new();
        for voice in self.voices.iter().filter(|voice| !voice.finished) {
            *active.entry(voice.context.clone()).or_default() += 1;
        }
        active
    }
    /// Fill interleaved stereo frames for one output at 48 kHz. Other outputs do
    /// not advance. Any odd trailing sample is silent and does not advance a frame.
    pub fn mix(&mut self, output: &str, samples: &mut [f32]) {
        samples.fill(0.0);
        for frame in samples.as_chunks_mut::<2>().0 {
            let mut sum = [0.0f64; 2];
            for voice in self
                .voices
                .iter_mut()
                .filter(|voice| !voice.finished && voice.config.output == output)
            {
                if let Some(value) = voice.next_frame() {
                    sum[0] += value[0];
                    sum[1] += value[1];
                }
            }
            for channel in 0..2 {
                frame[channel] = if sum[channel].is_finite() {
                    sum[channel].clamp(-1.0, 1.0) as f32
                } else {
                    0.0
                };
            }
        }
        self.voices.retain(|voice| !voice.finished);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{io::Write, path::Path};

    fn wav(path: &Path, rate: u32, channels: u16, samples: &[f32]) {
        let mut file = File::create(path).unwrap();
        let data_bytes = (samples.len() * 4) as u32;
        file.write_all(b"RIFF").unwrap();
        file.write_all(&(36 + data_bytes).to_le_bytes()).unwrap();
        file.write_all(b"WAVEfmt ").unwrap();
        for part in [
            16u32.to_le_bytes().to_vec(),
            3u16.to_le_bytes().to_vec(),
            channels.to_le_bytes().to_vec(),
            rate.to_le_bytes().to_vec(),
            (rate * u32::from(channels) * 4).to_le_bytes().to_vec(),
            (channels * 4).to_le_bytes().to_vec(),
            32u16.to_le_bytes().to_vec(),
        ] {
            file.write_all(&part).unwrap();
        }
        file.write_all(b"data").unwrap();
        file.write_all(&data_bytes.to_le_bytes()).unwrap();
        for sample in samples {
            file.write_all(&sample.to_le_bytes()).unwrap();
        }
    }
    fn config(path: &Path, mode: u8, fade: u8) -> Config {
        Config::parse(
            &json!({"path":path,"actionType":mode,"fadeType":fade,"fadeLen":1,"volume":100}),
        )
        .unwrap()
    }
    fn constant_file(frames: usize) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.wav");
        wav(&path, SAMPLE_RATE, 1, &vec![0.5; frames]);
        (dir, path)
    }
    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.002,
            "actual={actual}, expected={expected}"
        );
    }

    #[test]
    fn config_has_recovered_defaults_and_explicit_output_override() {
        let cfg = Config::parse(&json!({"path":"audio.wav"})).unwrap();
        assert_eq!(cfg.mode, Mode::PlayStop);
        assert_eq!(cfg.gain, 0.5);
        assert_eq!(cfg.fade_frames, 48000);
        assert_eq!(cfg.output, "");
        assert!(Config::parse(&json!({"path":"audio.wav","outputType":"Windows device"})).is_err());
        assert_eq!(
            Config::parse(
                &json!({"path":"audio.wav","outputType":"Windows device","linuxOutput":""})
            )
            .unwrap()
            .output,
            ""
        );
        assert_eq!(
            Config::parse(&json!({"path":"audio.wav","linuxOutput":"sink.monitor"}))
                .unwrap()
                .output,
            "sink.monitor"
        );
        for (key, value) in [
            ("actionType", json!(4)),
            ("fadeType", json!(4)),
            ("fadeLen", json!(0)),
            ("fadeLen", json!(6)),
            ("volume", json!(101)),
            ("volume", json!(-1)),
        ] {
            let mut settings = json!({"path":"audio.wav"});
            settings[key] = value;
            assert!(Config::parse(&settings).is_err());
        }
    }
    #[test]
    fn overlap_toggles_restart_and_failed_replacement_preserve_contexts() {
        let (_dir, path) = constant_file(100);
        let mut mixer = Mixer::new();
        mixer.trigger("a", config(&path, 0, 0)).unwrap();
        mixer.trigger("a", config(&path, 0, 0)).unwrap();
        assert_eq!(mixer.active()["a"], 2);
        let mut samples = [0.0; 4];
        mixer.mix("", &mut samples);
        close(samples[0], 1.0);
        mixer.trigger("b", config(&path, 1, 0)).unwrap();
        let mut invalid = config(&path, 2, 0);
        invalid.path = path.with_extension("missing");
        assert!(mixer.trigger("a", invalid).is_err());
        assert_eq!(mixer.active()["a"], 2);
        let invalid_path = path.with_extension("invalid");
        std::fs::write(&invalid_path, b"not a decodable audio file").unwrap();
        let mut invalid = config(&path, 2, 0);
        invalid.path = invalid_path;
        assert!(mixer.trigger("a", invalid).is_err());
        assert_eq!(mixer.active()["a"], 2);
        mixer.trigger("a", config(&path, 2, 0)).unwrap();
        assert_eq!(mixer.active()["a"], 1);
        assert_eq!(mixer.active()["b"], 1);
        mixer.trigger("a", config(&path, 1, 0)).unwrap();
        assert!(!mixer.active().contains_key("a"));
        assert!(mixer.active().contains_key("b"));
        mixer.stop_all();
        assert!(mixer.active().is_empty());
    }
    #[test]
    fn stereo_conversion_resampling_and_output_routing_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mono24k.wav");
        wav(&path, 24_000, 1, &vec![0.25; 240]);
        let mut mixer = Mixer::new();
        mixer.trigger("default", config(&path, 1, 0)).unwrap();
        let mut other = config(&path, 1, 0);
        other.output = "other".into();
        mixer.trigger("secondary", other).unwrap();
        let mut samples = vec![9.0; 962];
        mixer.mix("", &mut samples);
        close(samples[0], 0.25);
        close(samples[1], 0.25);
        assert!(mixer.active().contains_key("secondary"));
        assert!(!mixer.active().contains_key("default"));
        let mut secondary = [0.0; 4];
        mixer.mix("other", &mut secondary);
        close(secondary[0], 0.25);
        mixer.remove_output("other");
        assert!(mixer.outputs().is_empty());
    }
    #[test]
    fn natural_end_fade_reaches_zero_and_short_clip_envelope_is_bounded() {
        let (_dir, path) = constant_file(48000);
        let mut mixer = Mixer::new();
        mixer.trigger("fade", config(&path, 1, 2)).unwrap();
        let mut output = vec![0.0; 96000];
        mixer.mix("", &mut output);
        close(output[0], 0.5);
        close(output[48000], 0.25);
        close(output[95998], 0.0);
        assert!(mixer.active().is_empty());
        let (_shortdir, short) = constant_file(10);
        mixer.trigger("short", config(&short, 1, 3)).unwrap();
        let mut output = [0.0; 20];
        mixer.mix("", &mut output);
        close(output[0], 0.0);
        close(output[18], 0.0);
        assert!(output.iter().all(|&s| (0.0..=0.5).contains(&s)));
    }
    #[test]
    fn loop_is_continuous_fades_in_once_and_toggle_fades_out_once() {
        let (_dir, path) = constant_file(48000);
        let mut mixer = Mixer::new();
        mixer.trigger("loop", config(&path, 3, 3)).unwrap();
        let mut output = vec![0.0; 96004];
        mixer.mix("", &mut output);
        close(output[0], 0.0);
        close(output[48000], 0.25);
        close(output[95998], 0.5);
        close(output[96000], 0.5);
        mixer.trigger("loop", config(&path, 3, 3)).unwrap();
        let mut first = vec![0.0; 48000];
        mixer.mix("", &mut first);
        close(first[0], 0.5);
        close(first[47998], 0.25);
        mixer.stop_context("loop");
        let mut second = vec![0.0; 48000];
        mixer.mix("", &mut second);
        close(second[0], 0.25);
        close(second[47998], 0.0);
        assert!(mixer.active().is_empty());
    }
    #[test]
    fn stop_during_fade_in_does_not_increase_level_and_stop_all_is_immediate() {
        let (_dir, path) = constant_file(48000);
        let mut mixer = Mixer::new();
        mixer.trigger("a", config(&path, 3, 3)).unwrap();
        mixer.mix("", &mut vec![0.0; 24000]);
        mixer.stop_context("a");
        let mut output = vec![0.0; 2000];
        mixer.mix("", &mut output);
        assert!(output[0] <= 0.126);
        assert!(
            output
                .as_chunks::<2>()
                .0
                .iter()
                .map(|f| f[0])
                .collect::<Vec<_>>()
                .windows(2)
                .all(|pair| pair[1] <= pair[0])
        );
        mixer.stop_all();
        let mut output = [1.0; 8];
        mixer.mix("", &mut output);
        assert_eq!(output, [0.0; 8]);
    }
    #[test]
    fn manual_stop_near_natural_end_reaches_zero_before_the_file_ends() {
        let (_dir, path) = constant_file(48000);
        let mut mixer = Mixer::new();
        mixer.trigger("a", config(&path, 1, 2)).unwrap();
        mixer.mix("", &mut vec![0.0; 72000]);
        mixer.stop_context("a");
        let mut tail = vec![0.0; 24000];
        mixer.mix("", &mut tail);
        close(tail[0], 0.125);
        close(tail[23998], 0.0);
        assert!(mixer.active().is_empty());
    }
    #[test]
    fn invalid_float_samples_are_silent_and_final_mix_clips_without_order_bias() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("floats.wav");
        wav(
            &path,
            SAMPLE_RATE,
            2,
            &[f32::NAN, f32::INFINITY, 2.0, -2.0, 0.5, 0.5],
        );
        let mut mixer = Mixer::new();
        mixer.trigger("float", config(&path, 1, 0)).unwrap();
        let mut samples = [3.0; 7];
        mixer.mix("", &mut samples);
        assert_eq!(samples, [0.0, 0.0, 1.0, -1.0, 0.5, 0.5, 0.0]);
        let positive = dir.path().join("positive.wav");
        let negative = dir.path().join("negative.wav");
        wav(&positive, SAMPLE_RATE, 1, &[1.0; 4]);
        wav(&negative, SAMPLE_RATE, 1, &[-1.0; 4]);
        for (context, path) in [("a", &positive), ("b", &positive), ("c", &negative)] {
            mixer.trigger(context, config(path, 1, 0)).unwrap();
        }
        let mut output = [0.0; 2];
        mixer.mix("", &mut output);
        assert_eq!(output, [1.0; 2]);
    }
    #[test]
    fn voice_limits_and_sink_failure_do_not_silence_other_outputs() {
        let (_dir, path) = constant_file(32);
        let mut mixer = Mixer::new();
        for index in 0..MAX_CONTEXT_VOICES {
            let mut cfg = config(&path, 0, 0);
            cfg.output = if index % 2 == 0 { "one" } else { "two" }.into();
            mixer.trigger("overlap", cfg).unwrap();
        }
        assert!(mixer.trigger("overlap", config(&path, 0, 0)).is_err());
        assert_eq!(mixer.active()["overlap"], 8);
        mixer.remove_output("one");
        assert_eq!(mixer.active()["overlap"], 4);
        assert_eq!(mixer.outputs(), vec!["two"]);
        for index in 0..28 {
            mixer
                .trigger(&format!("ctx{index}"), config(&path, 0, 0))
                .unwrap();
        }
        assert!(mixer.trigger("extra", config(&path, 0, 0)).is_err());
        assert_eq!(mixer.active().values().sum::<usize>(), MAX_VOICES);
    }
    #[test]
    fn empty_loop_source_is_rejected_without_starting_a_voice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.wav");
        wav(&path, SAMPLE_RATE, 1, &[]);
        let mut mixer = Mixer::new();
        assert!(mixer.trigger("empty", config(&path, 3, 0)).is_err());
        assert!(mixer.active().is_empty());
    }
}
