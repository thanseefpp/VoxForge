//! VoxForge — Audio capture module.
//! Records audio using the device's preferred config, resamples to 16kHz mono f32.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

pub struct AudioRecorder {
    buffer: Arc<Mutex<Vec<f32>>>,
    stream: Option<cpal::Stream>,
    recording: Arc<std::sync::atomic::AtomicBool>,
}

unsafe impl Send for AudioRecorder {}
unsafe impl Sync for AudioRecorder {}

impl AudioRecorder {
    pub fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Vec::new())),
            stream: None,
            recording: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn start_recording(&mut self) -> Result<(), String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("No input device found")?;

        // Use the device's default input config instead of hardcoding
        let default_config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get default input config: {}", e))?;

        let sample_rate = default_config.sample_rate().0;
        let channels = default_config.channels() as usize;

        eprintln!(
            "[VoxForge] Device config: {}Hz, {} channels, {:?}",
            sample_rate, channels, default_config.sample_format()
        );

        let config: cpal::StreamConfig = default_config.into();

        // Clear previous buffer
        {
            let mut buf = self.buffer.lock().unwrap();
            buf.clear();
        }

        let buffer = self.buffer.clone();
        let recording = self.recording.clone();
        recording.store(true, std::sync::atomic::Ordering::SeqCst);

        // Resample ratio: device rate → 16kHz
        let resample_ratio = 16000.0 / sample_rate as f64;

        let stream = device
            .build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if !recording.load(std::sync::atomic::Ordering::SeqCst) {
                        return;
                    }
                    if let Ok(mut buf) = buffer.try_lock() {
                        // Convert to mono (take first channel) and resample to 16kHz
                        let mono: Vec<f32> = data
                            .chunks(channels)
                            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                            .collect();

                        if sample_rate == 16000 {
                            buf.extend_from_slice(&mono);
                        } else {
                            // Simple linear resampling
                            let out_len = (mono.len() as f64 * resample_ratio) as usize;
                            for i in 0..out_len {
                                let src_idx = i as f64 / resample_ratio;
                                let idx = src_idx as usize;
                                let frac = (src_idx - idx as f64) as f32;
                                let s0 = mono.get(idx).copied().unwrap_or(0.0);
                                let s1 = mono.get(idx + 1).copied().unwrap_or(s0);
                                buf.push(s0 + (s1 - s0) * frac);
                            }
                        }
                    }
                },
                |err| eprintln!("[VoxForge] Audio stream error: {}", err),
                None,
            )
            .map_err(|e| format!("Failed to build input stream: {}", e))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start stream: {}", e))?;

        self.stream = Some(stream);
        eprintln!("[VoxForge] Recording started (resampling {}Hz → 16kHz)", sample_rate);
        Ok(())
    }

    pub fn stop_recording(&mut self) -> Vec<f32> {
        self.recording
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.stream = None;

        let buf = self.buffer.lock().unwrap();
        eprintln!(
            "[VoxForge] Recording stopped — {:.1}s of audio",
            buf.len() as f64 / 16000.0
        );
        buf.clone()
    }

    pub fn is_recording(&self) -> bool {
        self.recording
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn get_buffer_snapshot(&self) -> Vec<f32> {
        self.buffer.lock().unwrap().clone()
    }

    /// Returns the current audio energy level (0.0 to 1.0)
    /// Used by the frontend to drive waveform visualization.
    pub fn get_audio_level(&self) -> f32 {
        let buf = self.buffer.lock().unwrap();
        if buf.len() < 256 {
            return 0.0;
        }
        // RMS of last 256 samples
        let tail = &buf[buf.len() - 256..];
        let rms = (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt();
        (rms * 10.0).min(1.0) // Scale and clamp
    }
}
