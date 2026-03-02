//! VoxForge — Whisper offline transcription module.
//!
//! Downloads whisper.cpp GGML models on demand from HuggingFace,
//! then transcribes audio buffers using whisper-rs with Metal acceleration.

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use tauri::Emitter;

/// Available Whisper models
pub const MODELS: &[(&str, &str, &str)] = &[
    (
        "ggml-small-q8_0.bin",
        "Small Q8 (~180MB)",
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q8_0.bin",
    ),
    (
        "ggml-large-v3-turbo-q8_0.bin",
        "Large v3 Turbo Q8 (~810MB)",
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q8_0.bin",
    ),
];

/// Get the models directory (~/.voxforge/models/)
fn models_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".voxforge").join("models")
}

/// Get the path to a specific model file
fn model_path(model_name: &str) -> PathBuf {
    models_dir().join(model_name)
}

/// Check if a model is already downloaded
pub fn is_model_downloaded(model_name: &str) -> bool {
    let path = model_path(model_name);
    path.exists() && path.metadata().map(|m| m.len() > 1_000_000).unwrap_or(false)
}

/// Download a model from HuggingFace with progress events.
/// Only downloads the selected model — not both.
pub fn ensure_model(
    model_name: &str,
    app: &tauri::AppHandle,
) -> Result<PathBuf, String> {
    let path = model_path(model_name);

    if is_model_downloaded(model_name) {
        eprintln!("[VoxForge] Whisper model already cached: {}", model_name);
        return Ok(path);
    }

    // Find URL for this model
    let url = MODELS
        .iter()
        .find(|(name, _, _)| *name == model_name)
        .map(|(_, _, url)| *url)
        .ok_or_else(|| format!("Unknown model: {}", model_name))?;

    eprintln!("[VoxForge] Downloading Whisper model: {} from {}", model_name, url);
    let _ = app.emit("model-download", serde_json::json!({
        "status": "starting",
        "model": model_name,
        "progress": 0,
    }));

    // Create models directory
    fs::create_dir_all(models_dir())
        .map_err(|e| format!("Failed to create models dir: {}", e))?;

    // Download with progress tracking
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3600))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("Download request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Download failed: HTTP {}", resp.status()));
    }

    let total_size = resp.content_length().unwrap_or(0);
    eprintln!("[VoxForge] Model size: {} MB", total_size / 1_000_000);

    // Stream to temp file, then rename
    let tmp_path = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp_path)
        .map_err(|e| format!("Failed to create temp file: {}", e))?;

    let mut downloaded: u64 = 0;
    let mut last_progress: u64 = 0;
    let mut reader = resp;

    loop {
        let mut buf = vec![0u8; 256 * 1024]; // 256KB chunks
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Download read error: {}", e))?;

        if n == 0 {
            break;
        }

        file.write_all(&buf[..n])
            .map_err(|e| format!("Write error: {}", e))?;

        downloaded += n as u64;

        // Emit progress every ~2%
        let progress = if total_size > 0 {
            (downloaded * 100 / total_size) as u64
        } else {
            0
        };

        if progress > last_progress + 1 {
            last_progress = progress;
            let _ = app.emit("model-download", serde_json::json!({
                "status": "downloading",
                "model": model_name,
                "progress": progress,
                "downloaded_mb": downloaded / 1_000_000,
                "total_mb": total_size / 1_000_000,
            }));
            eprintln!(
                "[VoxForge] Download: {}% ({}/{}MB)",
                progress,
                downloaded / 1_000_000,
                total_size / 1_000_000
            );
        }
    }

    file.flush().map_err(|e| format!("Flush error: {}", e))?;
    drop(file);

    // Rename temp → final
    fs::rename(&tmp_path, &path)
        .map_err(|e| format!("Rename error: {}", e))?;

    let _ = app.emit("model-download", serde_json::json!({
        "status": "done",
        "model": model_name,
        "progress": 100,
    }));

    eprintln!("[VoxForge] Model downloaded: {}", model_name);
    Ok(path)
}

/// Transcribe audio samples using whisper-rs.
/// `samples` should be f32 PCM at 16kHz mono.
pub fn transcribe(
    samples: &[f32],
    model_name: &str,
    app: &tauri::AppHandle,
) -> Result<String, String> {
    let model_path = ensure_model(model_name, app)?;

    eprintln!(
        "[VoxForge] Whisper transcribing {:.1}s of audio with {}",
        samples.len() as f64 / 16000.0,
        model_name
    );

    let _ = app.emit("pipeline-status", "transcribing");

    // Initialize whisper context with Metal
    let ctx = whisper_rs::WhisperContext::new_with_params(
        model_path.to_str().unwrap(),
        whisper_rs::WhisperContextParameters::default(),
    )
    .map_err(|e| format!("Whisper init error: {:?}", e))?;

    let mut state = ctx
        .create_state()
        .map_err(|e| format!("Whisper state error: {:?}", e))?;

    // Configure whisper parameters
    let mut params = whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_print_special(false);
    params.set_suppress_blank(true);
    params.set_suppress_non_speech_tokens(true);

    // Run inference
    state
        .full(params, samples)
        .map_err(|e| format!("Whisper inference error: {:?}", e))?;

    // Collect segments
    let num_segments = state.full_n_segments()
        .map_err(|e| format!("Segment count error: {:?}", e))?;

    let mut text = String::new();
    for i in 0..num_segments {
        if let Ok(segment_text) = state.full_get_segment_text(i) {
            text.push_str(segment_text.trim());
            text.push(' ');
        }
    }

    let result = text.trim().to_string();
    eprintln!("[VoxForge] Whisper result: \"{}\"", result);
    Ok(result)
}
