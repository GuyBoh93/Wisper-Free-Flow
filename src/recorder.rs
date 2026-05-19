// Audio capture via cpal. Captures from the default input device at whatever
// rate the device prefers, accumulates samples into a shared buffer, and
// resamples to 16kHz mono f32 (whisper's native input) when stopped.

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use std::sync::Arc;

/// True if cpal can see a default input device right now. Used to flash the
/// "no mic" overlay instead of silently failing on machines without a mic
/// (or where the user has unplugged / disabled the only one).
pub fn has_input_device() -> bool {
    cpal::default_host().default_input_device().is_some()
}

pub struct Recorder {
    stream: Option<cpal::Stream>,
    buffer: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            stream: None,
            buffer: Arc::new(Mutex::new(Vec::with_capacity(16_000 * 30))),
            sample_rate: 0,
        }
    }

    pub fn start(&mut self) -> Result<()> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("no default input device")?;
        let supported = device.default_input_config()?;

        let sample_format = supported.sample_format();
        let sample_rate = supported.sample_rate().0;
        let channels = supported.channels();
        let config: cpal::StreamConfig = supported.into();

        tracing::debug!(
            "audio: {} @ {}Hz {}ch ({:?})",
            device.name().unwrap_or_else(|_| "?".into()),
            sample_rate,
            channels,
            sample_format
        );

        self.sample_rate = sample_rate;
        self.buffer.lock().clear();

        let buffer = self.buffer.clone();
        let err_fn = |err| tracing::error!("audio stream error: {err}");

        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _: &_| ingest_f32(data, channels, &buffer),
                err_fn,
                None,
            )?,
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config,
                move |data: &[i16], _: &_| ingest_i16(data, channels, &buffer),
                err_fn,
                None,
            )?,
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config,
                move |data: &[u16], _: &_| ingest_u16(data, channels, &buffer),
                err_fn,
                None,
            )?,
            other => return Err(anyhow!("unsupported sample format: {other:?}")),
        };

        stream.play()?;
        self.stream = Some(stream);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<RecordedAudio> {
        self.stream.take();
        let samples = std::mem::take(&mut *self.buffer.lock());
        Ok(RecordedAudio {
            samples,
            sample_rate: self.sample_rate,
        })
    }

    pub fn is_recording(&self) -> bool {
        self.stream.is_some()
    }

    pub fn current_level(&self) -> f32 {
        let buf = self.buffer.lock();
        if buf.is_empty() {
            return 0.0;
        }
        let window = buf.len().saturating_sub(2048);
        let recent = &buf[window..];
        let sum_sq: f32 = recent.iter().map(|s| s * s).sum();
        (sum_sq / recent.len() as f32).sqrt()
    }
}

pub struct RecordedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl RecordedAudio {
    pub fn resample_to_16k(&self) -> Vec<f32> {
        const TARGET: u32 = 16_000;
        if self.sample_rate == TARGET || self.samples.is_empty() {
            return self.samples.clone();
        }
        let ratio = TARGET as f32 / self.sample_rate as f32;
        let out_len = (self.samples.len() as f32 * ratio) as usize;
        let mut out = Vec::with_capacity(out_len);
        let last = self.samples.len() - 1;
        for i in 0..out_len {
            let src = i as f32 / ratio;
            let lo = (src.floor() as usize).min(last);
            let hi = (lo + 1).min(last);
            let frac = src - lo as f32;
            out.push(self.samples[lo] * (1.0 - frac) + self.samples[hi] * frac);
        }
        out
    }

    pub fn duration_secs(&self) -> f32 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.samples.len() as f32 / self.sample_rate as f32
        }
    }
}

fn ingest_f32(data: &[f32], channels: u16, buffer: &Mutex<Vec<f32>>) {
    let mut buf = buffer.lock();
    if channels == 1 {
        buf.extend_from_slice(data);
    } else {
        for frame in data.chunks(channels as usize) {
            let sum: f32 = frame.iter().sum();
            buf.push(sum / channels as f32);
        }
    }
}

fn ingest_i16(data: &[i16], channels: u16, buffer: &Mutex<Vec<f32>>) {
    let mut buf = buffer.lock();
    let to_f32 = |s: i16| s as f32 / 32_768.0;
    if channels == 1 {
        buf.extend(data.iter().copied().map(to_f32));
    } else {
        for frame in data.chunks(channels as usize) {
            let sum: f32 = frame.iter().copied().map(to_f32).sum();
            buf.push(sum / channels as f32);
        }
    }
}

fn ingest_u16(data: &[u16], channels: u16, buffer: &Mutex<Vec<f32>>) {
    let mut buf = buffer.lock();
    let to_f32 = |s: u16| (s as f32 - 32_768.0) / 32_768.0;
    if channels == 1 {
        buf.extend(data.iter().copied().map(to_f32));
    } else {
        for frame in data.chunks(channels as usize) {
            let sum: f32 = frame.iter().copied().map(to_f32).sum();
            buf.push(sum / channels as f32);
        }
    }
}
