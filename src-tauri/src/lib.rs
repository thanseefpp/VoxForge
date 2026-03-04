//! VoxForge — Main application module.
//!
//! Wires up audio capture → Deepgram WebSocket STT → Groq LLM polish → paste.

mod audio;
mod deepgram;
mod focus;
mod groq;
mod paste;
mod whisper;

use audio::AudioRecorder;
use groq::PromptMode;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{Emitter, Manager};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri_plugin_global_shortcut::GlobalShortcutExt;

// ─── Settings ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppSettings {
    pub deepgram_api_key: String,
    pub groq_api_key: String,
    #[serde(default = "default_engine")]
    pub stt_engine: String,
    #[serde(default = "default_prompt_mode")]
    pub prompt_mode: String,
    pub custom_prompt: String,
    #[serde(default = "default_true")]
    pub auto_paste: bool,
    pub llm_enabled: bool,
    #[serde(default = "default_groq_model")]
    pub groq_model: String,
    #[serde(default = "default_whisper_model")]
    pub whisper_model: String,
    /// Last known logical X position of the overlay window. `None` means
    /// "use default bottom-center placement on first show".
    pub window_x: Option<f64>,
    /// Last known logical Y position of the overlay window. `None` means
    /// "use default bottom-center placement on first show".
    pub window_y: Option<f64>,
}

fn default_engine() -> String { "deepgram".to_string() }
fn default_prompt_mode() -> String { "direct".to_string() }
fn default_true() -> bool { true }
fn default_groq_model() -> String { "llama-3.1-8b-instant".to_string() }
fn default_whisper_model() -> String { "ggml-small-q8_0.bin".to_string() }

// ─── Settings Persistence ────────────────────────────────────────────────────

/// Returns the path to the settings file: `~/.voxforge/settings.json`.
/// Uses the same base directory as the Whisper model cache.
fn settings_file_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".voxforge").join("settings.json")
}

/// Load settings from `~/.voxforge/settings.json`.
/// Returns `AppSettings::default()` if the file is absent, unreadable, or malformed.
/// Never panics — all errors are logged with `eprintln!`.
fn load_settings_from_disk() -> AppSettings {
    let path = settings_file_path();
    if !path.exists() {
        return AppSettings::default();
    }

    match fs::read_to_string(&path) {
        Ok(contents) => match serde_json::from_str::<AppSettings>(&contents) {
            Ok(settings) => {
                eprintln!("[VoxForge] Settings loaded from {}", path.display());
                settings
            }
            Err(e) => {
                eprintln!("[VoxForge] settings.json is malformed, using defaults: {}", e);
                AppSettings::default()
            }
        },
        Err(e) => {
            eprintln!("[VoxForge] Could not read settings.json, using defaults: {}", e);
            AppSettings::default()
        }
    }
}

/// Persist settings to `~/.voxforge/settings.json`.
/// Writes to a sibling `.tmp` file first, then renames atomically so a crash
/// mid-write never corrupts the live settings file.
/// All errors are logged with `eprintln!`; the function never panics.
fn save_settings_to_disk(settings: &AppSettings) {
    let path = settings_file_path();

    // Ensure the ~/.voxforge/ directory exists
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            eprintln!("[VoxForge] Could not create settings dir {}: {}", parent.display(), e);
            return;
        }
    }

    let json = match serde_json::to_string_pretty(settings) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[VoxForge] Failed to serialize settings: {}", e);
            return;
        }
    };

    // Write to temp file then rename for atomicity
    let tmp_path = path.with_extension("tmp");
    match fs::File::create(&tmp_path) {
        Ok(mut file) => {
            if let Err(e) = file.write_all(json.as_bytes()) {
                eprintln!("[VoxForge] Failed to write settings temp file: {}", e);
                return;
            }
            if let Err(e) = file.flush() {
                eprintln!("[VoxForge] Failed to flush settings temp file: {}", e);
                return;
            }
        }
        Err(e) => {
            eprintln!("[VoxForge] Failed to create settings temp file: {}", e);
            return;
        }
    }

    if let Err(e) = fs::rename(&tmp_path, &path) {
        eprintln!("[VoxForge] Failed to rename settings temp file: {}", e);
        return;
    }

    eprintln!("[VoxForge] Settings persisted to {}", path.display());
}

// ─── App State ───────────────────────────────────────────────────────────────

pub struct AppState {
    pub recorder: Mutex<AudioRecorder>,
    pub settings: Mutex<AppSettings>,
    pub is_recording: AtomicBool,
    pub stop_streaming: Arc<AtomicBool>,
    /// Flipped on the first call to `show_overlay`. Subsequent calls skip
    /// repositioning so the user's dragged position is preserved.
    pub overlay_positioned: OnceLock<()>,
}

// ─── Core Recording Logic ────────────────────────────────────────────────────

/// Toggle recording on or off. Called by both the Tauri command and the global
/// hotkey handler so all pipeline logic lives in exactly one place.
fn do_toggle_recording(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let is_recording = state.is_recording.load(Ordering::SeqCst);

    if is_recording {
        // ── STOP ──
        eprintln!("[VoxForge] Stopping...");
        state.is_recording.store(false, Ordering::SeqCst);
        state.stop_streaming.store(true, Ordering::SeqCst);

        let audio = state.recorder.lock().unwrap().stop_recording();
        let _ = app.emit("pipeline-status", "processing");

        let settings = state.settings.lock().unwrap().clone();

        // Finalize in background thread
        let app_clone = app.clone();
        std::thread::spawn(move || {
            // ── Whisper offline mode ──
            if settings.stt_engine == "whisper" {
                eprintln!("[VoxForge] Running Whisper offline transcription...");
                let _ = app_clone.emit("pipeline-status", "transcribing");

                match whisper::transcribe(&audio, &settings.whisper_model, &app_clone) {
                    Ok(text) if !text.is_empty() => {
                        let _ = app_clone.emit("streaming-text", &text);

                        // Polish with Groq if LLM enabled
                        let final_text = if settings.llm_enabled && settings.prompt_mode != "direct" {
                            let mode = match settings.prompt_mode.as_str() {
                                "coding" => groq::PromptMode::Coding,
                                "email" => groq::PromptMode::Email,
                                "general" => groq::PromptMode::General,
                                "casual" => groq::PromptMode::Casual,
                                "custom" => groq::PromptMode::Custom,
                                _ => groq::PromptMode::Direct,
                            };
                            if mode != groq::PromptMode::Direct {
                                eprintln!("[VoxForge] Polishing Whisper output with Groq...");
                                let _ = app_clone.emit("pipeline-status", "polishing");
                                let polished = groq::polish_prompt(
                                    &settings.groq_api_key, &text, &mode,
                                    &settings.custom_prompt, &settings.groq_model,
                                );
                                let _ = app_clone.emit("streaming-text", &polished);
                                polished
                            } else {
                                text
                            }
                        } else {
                            text
                        };

                        // Paste result
                        if settings.auto_paste {
                            focus::restore_focus();
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            let _ = paste::paste_text(&final_text);
                        }
                    }
                    Ok(_) => eprintln!("[VoxForge] Whisper returned empty text"),
                    Err(e) => {
                        eprintln!("[VoxForge] Whisper error: {}", e);
                        let _ = app_clone.emit("pipeline-error", &e);
                    }
                }
            } else {
                // Deepgram: text was already streamed word-by-word
                focus::restore_focus();
                std::thread::sleep(std::time::Duration::from_millis(150));
            }

            // Emit done — the bubble will animate the checkmark then return to
            // idle (mic icon) on its own. The overlay window stays visible.
            let _ = app_clone.emit("pipeline-status", "done");
        });
    } else {
        // ── START ──
        eprintln!("[VoxForge] Starting...");
        state.is_recording.store(true, Ordering::SeqCst);
        state.stop_streaming.store(false, Ordering::SeqCst);

        match state.recorder.lock().unwrap().start_recording() {
            Ok(_) => {
                let _ = app.emit("pipeline-status", "recording");
                show_overlay(app);

                // Restore focus so typed text goes to target app
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    focus::restore_focus();
                });

                // Start streaming pipeline
                let settings = state.settings.lock().unwrap().clone();
                if !settings.deepgram_api_key.is_empty() && settings.stt_engine == "deepgram" {
                    start_deepgram_streaming(app.clone());
                }
            }
            Err(e) => {
                eprintln!("[VoxForge] Failed to start recording: {}", e);
                let _ = app.emit("pipeline-error", &e);
            }
        }
    }
}

// ─── Tauri Commands ──────────────────────────────────────────────────────────

#[tauri::command]
fn get_settings(state: tauri::State<'_, AppState>) -> AppSettings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(state: tauri::State<'_, AppState>, settings: AppSettings) {
    eprintln!("[VoxForge] Settings saved: engine={}, mode={}, llm={}, model={}",
        settings.stt_engine, settings.prompt_mode, settings.llm_enabled, settings.groq_model);
    *state.settings.lock().unwrap() = settings.clone();
    save_settings_to_disk(&settings);
}

#[tauri::command]
fn check_accessibility() -> bool {
    paste::check_accessibility()
}

#[tauri::command]
fn get_audio_level(state: tauri::State<'_, AppState>) -> f32 {
    state.recorder.lock().unwrap().get_audio_level()
}

#[tauri::command]
fn check_whisper_model_downloaded(model_name: String) -> bool {
    whisper::is_model_downloaded(&model_name)
}

#[tauri::command]
fn download_whisper_model(app: tauri::AppHandle, model_name: String) {
    std::thread::spawn(move || {
        match whisper::ensure_model(&model_name, &app) {
            Ok(_) => eprintln!("[VoxForge] Model ready: {}", model_name),
            Err(e) => {
                eprintln!("[VoxForge] Download failed: {}", e);
                let _ = app.emit("model-download", serde_json::json!({
                    "status": "error",
                    "model": model_name,
                    "error": e,
                }));
            }
        }
    });
}

/// Toggle recording on or off. This is the IPC entry point from the frontend;
/// the actual logic is in `do_toggle_recording`.
#[tauri::command]
fn toggle_recording(app: tauri::AppHandle) {
    do_toggle_recording(&app);
}

/// Persist the overlay window's current logical position to settings so it
/// survives across restarts. Called by the frontend after a drag ends.
#[tauri::command]
fn save_window_position(
    state: tauri::State<'_, AppState>,
    x: f64,
    y: f64,
) -> Result<(), String> {
    let mut settings = state.settings.lock().unwrap();
    settings.window_x = Some(x);
    settings.window_y = Some(y);
    save_settings_to_disk(&settings);
    Ok(())
}

// ─── Deepgram Streaming Pipeline ─────────────────────────────────────────────

// Smoothed audio level must stay below this for SILENCE_SECS to trigger auto-stop.
// 0.10 ≈ unscaled RMS 0.01 — above typical ambient noise, well below speech.
const SILENCE_THRESHOLD: f32 = 0.10;
const SILENCE_SECS: u64 = 2;

fn start_deepgram_streaming(app: tauri::AppHandle) {
    let state = app.state::<AppState>();
    let api_key = state.settings.lock().unwrap().deepgram_api_key.clone();
    let stop_flag = state.stop_streaming.clone();
    let settings = state.settings.lock().unwrap().clone();

    std::thread::Builder::new()
        .name("deepgram-stream".into())
        .spawn(move || {
            // Connect to Deepgram
            let mut client = match deepgram::DeepgramClient::connect(&api_key) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[VoxForge] Deepgram connect failed: {}", e);
                    let _ = app.emit("pipeline-error", &e);
                    return;
                }
            };

            eprintln!("[VoxForge] Deepgram streaming started");

            // Main loop: send audio + receive results in real-time
            let mut last_sent = 0usize;
            let mut accumulated_final = String::new();
            let mut send_counter = 0u64;
            let mut last_speech_time = std::time::Instant::now();
            let mut has_received_speech = false;
            let mut silence_triggered = false;
            // Exponential moving average of audio level (α=0.15, τ≈130ms at 20ms loop)
            // smooths out transient noise spikes so they don't reset the silence timer.
            let mut smooth_level: f32 = 0.0;

            loop {
                if stop_flag.load(Ordering::SeqCst) {
                    let _ = client.close();
                    break;
                }

                // ── Send audio chunk ──
                {
                    let state = app.state::<AppState>();
                    let buf = state.recorder.lock().unwrap().get_buffer_snapshot();
                    if buf.len() > last_sent {
                        let chunk = &buf[last_sent..];
                        last_sent = buf.len();
                        if let Err(e) = client.send_audio(chunk) {
                            eprintln!("[VoxForge] Send error: {}", e);
                            break;
                        }
                        send_counter += 1;
                        if send_counter % 50 == 1 {
                            eprintln!("[VoxForge] Sent {} chunks, {}s audio",
                                send_counter, last_sent / 16000);
                        }
                    }
                }

                // ── Receive all available results (drain queue) ──
                loop {
                    match client.try_recv() {
                        Ok(Some(result)) => {
                            if result.is_final {
                                // Final confirmed text — accumulate and paste
                                if !result.text.is_empty() {
                                    eprintln!("[VoxForge] FINAL: \"{}\"", result.text);
                                    accumulated_final.push_str(&result.text);
                                    accumulated_final.push(' ');
                                    let _ = app.emit("streaming-text", &accumulated_final.trim());

                                    // Only paste per-sentence when NOT using LLM polish.
                                    // When LLM is on, we accumulate everything and paste once after polish.
                                    let use_llm = settings.llm_enabled
                                        && settings.prompt_mode != "direct";
                                    if settings.auto_paste && !use_llm {
                                        focus::restore_focus();
                                        std::thread::sleep(std::time::Duration::from_millis(30));
                                        let _ = paste::paste_text(&result.text);
                                    }
                                }
                            } else {
                                // Interim result — show live preview only
                                if !result.text.is_empty() {
                                    let preview = format!("{}{}", accumulated_final, result.text);
                                    let _ = app.emit("streaming-text", &preview.trim());
                                }
                            }
                        }
                        Ok(None) => break, // No more messages available
                        Err(e) => {
                            eprintln!("[VoxForge] Recv error: {}", e);
                            break;
                        }
                    }
                }

                // ── Silence detection (Deepgram + LLM mode only) ──
                if settings.llm_enabled
                    && settings.prompt_mode != "direct"
                    && !accumulated_final.is_empty()
                    && !silence_triggered
                {
                    let raw = {
                        let st = app.state::<AppState>();
                        let lvl = st.recorder.lock().unwrap().get_audio_level();
                        lvl
                    };
                    // EMA smoothing: damps transient spikes (keyboard, HVAC, etc.)
                    smooth_level = smooth_level * 0.85 + raw * 0.15;

                    if smooth_level > SILENCE_THRESHOLD {
                        last_speech_time = std::time::Instant::now();
                        has_received_speech = true;
                    } else if has_received_speech
                        && last_speech_time.elapsed()
                            >= std::time::Duration::from_secs(SILENCE_SECS)
                    {
                        silence_triggered = true;
                        eprintln!("[VoxForge] Silence detected — auto-stopping");
                        let _ = app.emit("pipeline-status", "silence");
                        // Auto-stop: flip state flags + drop audio stream
                        {
                            let st = app.state::<AppState>();
                            st.is_recording.store(false, Ordering::SeqCst);
                            let _ = st.recorder.lock().unwrap().stop_recording();
                        }
                        stop_flag.store(true, Ordering::SeqCst);
                        let _ = client.close();
                        break;
                    }
                }

                // Small sleep to prevent busy-spinning
                std::thread::sleep(std::time::Duration::from_millis(20));
            }

            // Polish with Groq if LLM is enabled
            let final_text = accumulated_final.trim().to_string();
            if settings.llm_enabled && !final_text.is_empty() && settings.prompt_mode != "direct" {
                let mode = match settings.prompt_mode.as_str() {
                    "coding" => PromptMode::Coding,
                    "email" => PromptMode::Email,
                    "general" => PromptMode::General,
                    "casual" => PromptMode::Casual,
                    "custom" => PromptMode::Custom,
                    _ => PromptMode::Direct,
                };

                if mode != PromptMode::Direct {
                    eprintln!("[VoxForge] Polishing with Groq (model: {})...", settings.groq_model);
                    let _ = app.emit("pipeline-status", "polishing");
                    let polished = groq::polish_prompt(
                        &settings.groq_api_key,
                        &final_text,
                        &mode,
                        &settings.custom_prompt,
                        &settings.groq_model,
                    );
                    let _ = app.emit("streaming-text", &polished);
                    // Paste the polished version
                    focus::restore_focus();
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    let _ = paste::paste_text(&polished);
                }
            }
            // Only emit "done" from here on the silence auto-stop path.
            // On the normal user-stop path, do_toggle_recording already emits "done";
            // a second emit here would reset the hide timer unexpectedly.
            if silence_triggered {
                let _ = app.emit("pipeline-status", "done");
            }
            eprintln!("[VoxForge] Deepgram streaming ended");
        })
        .expect("Failed to spawn Deepgram thread");
}

// ─── Overlay Helpers ─────────────────────────────────────────────────────────

/// The logical size of the overlay window. Must match the value in `tauri.conf.json`.
const OVERLAY_SIZE_PX: f64 = 88.0;

/// Show the overlay window.
///
/// On the **first** call (per process lifetime) the window is positioned:
/// - At `(settings.window_x, settings.window_y)` if the user previously saved a
///   position, or
/// - At the bottom-center of the primary monitor otherwise.
///
/// On subsequent calls the position is left untouched so the user's dragged
/// location is preserved.
fn show_overlay(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window("overlay") else {
        return;
    };

    let state = app.state::<AppState>();

    // Only position on first show; after that honour the dragged position.
    if state.overlay_positioned.set(()).is_ok() {
        let saved = {
            let settings = state.settings.lock().unwrap();
            settings.window_x.zip(settings.window_y)
        };

        let position = match saved {
            Some((x, y)) => tauri::LogicalPosition::new(x, y),
            None => {
                // Default: bottom-center of the primary monitor.
                let (sw, sh, scale) = win
                    .primary_monitor()
                    .ok()
                    .flatten()
                    .map(|m| (m.size().width as f64, m.size().height as f64, m.scale_factor()))
                    .unwrap_or((1440.0, 900.0, 1.0));
                let x = (sw / scale - OVERLAY_SIZE_PX) / 2.0;
                let y = sh / scale - OVERLAY_SIZE_PX - 80.0;
                tauri::LogicalPosition::new(x, y)
            }
        };

        let _ = win.set_position(tauri::Position::Logical(position));
    }

    let _ = win.show();
}

/// Hide the overlay window and persist its current logical position to settings
/// so it is restored at the same spot on the next recording session.
fn hide_overlay(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window("overlay") else {
        return;
    };

    // Capture physical position and convert to logical coordinates.
    if let (Ok(phys_pos), Ok(scale)) = (win.outer_position(), win.scale_factor()) {
        if scale > 0.0 {
            let logical_x = phys_pos.x as f64 / scale;
            let logical_y = phys_pos.y as f64 / scale;

            let state = app.state::<AppState>();
            let mut settings = state.settings.lock().unwrap();
            settings.window_x = Some(logical_x);
            settings.window_y = Some(logical_y);
            save_settings_to_disk(&settings);
        }
    }

    let _ = win.hide();
}

// ─── Run ─────────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = AppState {
        recorder: Mutex::new(AudioRecorder::new()),
        settings: Mutex::new(load_settings_from_disk()),
        is_recording: AtomicBool::new(false),
        stop_streaming: Arc::new(AtomicBool::new(false)),
        overlay_positioned: OnceLock::new(),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            check_accessibility,
            get_audio_level,
            check_whisper_model_downloaded,
            download_whisper_model,
            toggle_recording,
            save_window_position,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // Start focus tracker
            focus::start_focus_tracker();

            // Check accessibility
            paste::check_accessibility();

            // Register Ctrl+Space global hotkey to toggle recording.
            // The handler fires on key-down only; key-up events are ignored so a
            // single physical key press produces exactly one toggle.
            app.handle()
                .global_shortcut()
                .on_shortcut(
                    "CTRL+Space",
                    |app_handle, _shortcut, event| {
                        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                            do_toggle_recording(app_handle);
                        }
                    },
                )
                .unwrap_or_else(|e| eprintln!("[VoxForge] Failed to register Ctrl+Space hotkey: {}", e));

            // Show the overlay bubble immediately on startup so it is always
            // visible as an idle mic button. `show_overlay` positions it once
            // (saved position or bottom-center) and is a no-op on later calls.
            {
                let handle = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    show_overlay(&handle);
                });
            }

            // System tray
            {
                use tauri::menu::{Menu, MenuItem};
                use tauri::tray::TrayIconBuilder;

                let show_settings = MenuItem::with_id(app, "show_settings", "Settings", true, None::<&str>)?;
                let quit_item = MenuItem::with_id(app, "quit", "Quit VoxForge", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show_settings, &quit_item])?;

                let icon_bytes = include_bytes!("../icons/32x32.png");
                let icon = tauri::image::Image::from_bytes(icon_bytes)?;

                let tray_handle = handle.clone();
                TrayIconBuilder::new()
                    .icon(icon)
                    .tooltip("VoxForge — AI Voice-to-Text")
                    .menu(&menu)
                    .on_menu_event(move |_tray, event| {
                        match event.id.as_ref() {
                            "show_settings" => {
                                if let Some(win) = tray_handle.get_webview_window("main") {
                                    let _ = win.show();
                                    let _ = win.set_focus();
                                }
                            }
                            "quit" => std::process::exit(0),
                            _ => {}
                        }
                    })
                    .build(app)?;
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running VoxForge");
}
