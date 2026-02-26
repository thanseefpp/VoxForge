//! VoxForge — Main application module.
//!
//! Wires up audio capture → Deepgram WebSocket STT → Groq LLM polish → paste.

mod audio;
mod deepgram;
mod focus;
mod groq;
mod paste;

use audio::AudioRecorder;
use groq::PromptMode;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

// ─── Settings ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub deepgram_api_key: String,
    pub groq_api_key: String,
    pub stt_engine: String,  // "deepgram" | "whisper"
    pub prompt_mode: String, // "direct" | "coding" | "email" | "general" | "custom"
    pub custom_prompt: String,
    pub auto_paste: bool,
    pub llm_enabled: bool,
    pub groq_model: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            deepgram_api_key: String::new(),
            groq_api_key: String::new(),
            stt_engine: "deepgram".to_string(),
            prompt_mode: "direct".to_string(),
            custom_prompt: String::new(),
            auto_paste: true,
            llm_enabled: false,
            groq_model: "llama-3.1-8b-instant".to_string(),
        }
    }
}

// ─── App State ───────────────────────────────────────────────────────────────

pub struct AppState {
    pub recorder: Mutex<AudioRecorder>,
    pub settings: Mutex<AppSettings>,
    pub is_recording: AtomicBool,
    pub stop_streaming: Arc<AtomicBool>,
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
    *state.settings.lock().unwrap() = settings;
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
fn toggle_recording(app: tauri::AppHandle) {
    let state = app.state::<AppState>();
    let is_recording = state.is_recording.load(Ordering::SeqCst);

    if is_recording {
        // ── STOP ──
        eprintln!("[VoxForge] Stopping...");
        state.is_recording.store(false, Ordering::SeqCst);
        state.stop_streaming.store(true, Ordering::SeqCst);

        let _audio = state.recorder.lock().unwrap().stop_recording();
        let _ = app.emit("pipeline-status", "processing");

        let settings = state.settings.lock().unwrap().clone();

        // Finalize in background thread
        let app_clone = app.clone();
        std::thread::spawn(move || {
            // Restore focus before pasting
            focus::restore_focus();
            std::thread::sleep(std::time::Duration::from_millis(150));

            // If using whisper offline (no Deepgram key), transcribe the buffer
            if settings.stt_engine == "whisper" || settings.deepgram_api_key.is_empty() {
                eprintln!("[VoxForge] Offline finalization skipped (Deepgram handled streaming)");
            }

            // Deepgram: text was already streamed and pasted word-by-word
            // Just emit done
            let _ = app_clone.emit("pipeline-status", "done");
            std::thread::sleep(std::time::Duration::from_millis(1200));
            hide_overlay(&app_clone);
        });
    } else {
        // ── START ──
        eprintln!("[VoxForge] Starting...");
        state.is_recording.store(true, Ordering::SeqCst);
        state.stop_streaming.store(false, Ordering::SeqCst);

        match state.recorder.lock().unwrap().start_recording() {
            Ok(_) => {
                let _ = app.emit("pipeline-status", "recording");
                show_overlay(&app);

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

// ─── Deepgram Streaming Pipeline ─────────────────────────────────────────────

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

                                    // Auto-paste confirmed sentences
                                    if settings.auto_paste {
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
            eprintln!("[VoxForge] Deepgram streaming ended");
        })
        .expect("Failed to spawn Deepgram thread");
}

// ─── Overlay Helpers ─────────────────────────────────────────────────────────

fn show_overlay(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("overlay") {
        // Position at bottom-center of primary monitor
        if let Some(monitor) = win.primary_monitor().ok().flatten() {
            let screen = monitor.size();
            let scale = monitor.scale_factor();
            let w = 400.0;
            let h = 300.0;
            let x = (screen.width as f64 / scale - w) / 2.0;
            let y = screen.height as f64 / scale - h - 80.0;

            let _ = win.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)));
        }
        let _ = win.show();
    }
}

fn hide_overlay(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("overlay") {
        let _ = win.hide();
    }
}

// ─── Run ─────────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = AppState {
        recorder: Mutex::new(AudioRecorder::new()),
        settings: Mutex::new(AppSettings::default()),
        is_recording: AtomicBool::new(false),
        stop_streaming: Arc::new(AtomicBool::new(false)),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            check_accessibility,
            get_audio_level,
            toggle_recording,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // Start focus tracker
            focus::start_focus_tracker();

            // Check accessibility
            paste::check_accessibility();

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

            // Show overlay after a delay
            let overlay_handle = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                show_overlay(&overlay_handle);
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running VoxForge");
}
