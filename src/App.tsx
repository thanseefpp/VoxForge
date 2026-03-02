import { useState, useEffect } from "react";
import "./App.css";

// ─── Tauri invoke helper ─────────────────────────────────────────────────────
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T | undefined> {
  try {
    const { invoke: tauriInvoke } = await import("@tauri-apps/api/core");
    return await tauriInvoke<T>(cmd, args);
  } catch {
    return undefined;
  }
}

interface Settings {
  deepgram_api_key: string;
  groq_api_key: string;
  stt_engine: string;
  prompt_mode: string;
  custom_prompt: string;
  auto_paste: boolean;
  llm_enabled: boolean;
  groq_model: string;
  whisper_model: string;
}

const defaultSettings: Settings = {
  deepgram_api_key: "",
  groq_api_key: "",
  stt_engine: "deepgram",
  prompt_mode: "direct",
  custom_prompt: "",
  auto_paste: true,
  llm_enabled: false,
  groq_model: "llama-3.1-8b-instant",
  whisper_model: "ggml-small-q8_0.bin",
};

export default function App() {
  const [settings, setSettings] = useState<Settings>(defaultSettings);
  const [saved, setSaved] = useState(false);
  const [accessGranted, setAccessGranted] = useState(false);

  // Track download status per model: "idle" | "downloading" | "done" | "error"
  const [modelStatus, setModelStatus] = useState<Record<string, {
    status: string;
    progress: number;
    downloaded_mb?: number;
    total_mb?: number;
  }>>({});

  // Check which models are already downloaded on mount
  useEffect(() => {
    (async () => {
      const s = await invoke<Settings>("get_settings");
      if (s) setSettings(s);
      const a = await invoke<boolean>("check_accessibility");
      if (a !== undefined) setAccessGranted(a);

      // Check each Whisper model download status
      for (const modelName of ["ggml-small-q8_0.bin", "ggml-large-v3-turbo-q8_0.bin"]) {
        const downloaded = await invoke<boolean>("check_whisper_model_downloaded", { modelName });
        if (downloaded) {
          setModelStatus(prev => ({ ...prev, [modelName]: { status: "done", progress: 100 } }));
        }
      }

      // Listen for model download progress events
      try {
        const { listen } = await import("@tauri-apps/api/event");
        listen<any>("model-download", (e: any) => {
          const p = e.payload;
          const model = p.model as string;
          setModelStatus(prev => ({
            ...prev,
            [model]: {
              status: p.status,
              progress: p.progress || 0,
              downloaded_mb: p.downloaded_mb,
              total_mb: p.total_mb,
            },
          }));
        });
      } catch { }
    })();
  }, []);

  const update = (key: keyof Settings, value: string | boolean) => {
    setSettings((prev) => ({ ...prev, [key]: value }));
    setSaved(false);
  };

  const handleSave = async () => {
    await invoke("save_settings", { settings });
    setSaved(true);
    setTimeout(() => setSaved(false), 2000);
  };

  return (
    <div className="app-container">
      {/* ── Header ── */}
      <header className="app-header">
        <div className="app-logo">
          <svg viewBox="0 0 24 24" fill="none" stroke="#fff" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <rect x="9" y="2" width="6" height="12" rx="3" />
            <path d="M5 10a7 7 0 0 0 14 0" />
            <line x1="12" y1="18" x2="12" y2="22" />
          </svg>
        </div>
        <h1 className="app-title">VoxForge</h1>
        <p className="app-subtitle">Real-time AI Voice-to-Text</p>
      </header>

      {/* ── Status Bar ── */}
      <div className="status-bar">
        <div className="status-pill">
          <div className={`status-dot ${settings.deepgram_api_key ? "connected" : "disconnected"}`} />
          Deepgram
        </div>
        <div className="status-pill">
          <div className={`status-dot ${settings.groq_api_key ? "connected" : "disconnected"}`} />
          Groq LLM
        </div>
        <div className="status-pill">
          <div className={`status-dot ${accessGranted ? "connected" : "error"}`} />
          Access
        </div>
      </div>

      {/* ── Section 1: API Keys ── */}
      <div className="section">
        <div className="section-header">
          <span className="section-number">01</span>
          <span className="section-title">API Keys</span>
        </div>

        <div className="card">
          <div className="card-label">Deepgram API Key</div>
          <div className="card-desc">For real-time voice streaming. Get one free at deepgram.com</div>
          <input
            type="password"
            className="input-field"
            placeholder="dg_xxxxxxxxxxxxxxxxxxxxxxxx"
            value={settings.deepgram_api_key}
            onChange={(e) => update("deepgram_api_key", e.target.value)}
          />
        </div>

        <div className="card">
          <div className="card-label">Groq API Key</div>
          <div className="card-desc">For LLM prompt polishing. Free tier at groq.com</div>
          <input
            type="password"
            className="input-field"
            placeholder="gsk_xxxxxxxxxxxxxxxxxxxxxxxx"
            value={settings.groq_api_key}
            onChange={(e) => update("groq_api_key", e.target.value)}
          />
        </div>
      </div>

      {/* ── Section 2: STT Engine ── */}
      <div className="section">
        <div className="section-header">
          <span className="section-number">02</span>
          <span className="section-title">STT Engine</span>
        </div>

        <div className="toggle-group">
          <button
            className={`toggle-btn ${settings.stt_engine === "deepgram" ? "active" : ""}`}
            onClick={() => update("stt_engine", "deepgram")}
          >
            <div className="toggle-btn-icon">⚡</div>
            <div className="toggle-btn-name">Deepgram</div>
            <div className="toggle-btn-desc">Real-time (Nova-3)</div>
          </button>
          <button
            className={`toggle-btn ${settings.stt_engine === "whisper" ? "active" : ""}`}
            onClick={() => update("stt_engine", "whisper")}
          >
            <div className="toggle-btn-icon">🔒</div>
            <div className="toggle-btn-name">Whisper</div>
            <div className="toggle-btn-desc">Offline / local</div>
          </button>
        </div>

        {/* Whisper model selector — only shown when Whisper is selected */}
        {settings.stt_engine === "whisper" && (() => {
          const models = [
            { id: "ggml-small-q8_0.bin", name: "Small Q8", size: "~180 MB", desc: "Fast", icon: "⚡" },
            { id: "ggml-large-v3-turbo-q8_0.bin", name: "Large v3 Turbo", size: "~810 MB", desc: "Best accuracy", icon: "🧠" },
          ];
          return (
            <div className="card" style={{ marginTop: 10 }}>
              <div className="card-label">Whisper Model</div>
              <div className="card-desc">Click to download · Select a downloaded model to use</div>
              <div className="toggle-group" style={{ flexDirection: "column" }}>
                {models.map((m) => {
                  const st = modelStatus[m.id] || { status: "idle", progress: 0 };
                  const isDownloaded = st.status === "done";
                  const isDownloading = st.status === "downloading" || st.status === "starting";
                  const isSelected = settings.whisper_model === m.id && isDownloaded;
                  const isError = st.status === "error";

                  const handleClick = () => {
                    if (isDownloading) return; // Don't interfere
                    if (isDownloaded) {
                      update("whisper_model", m.id); // Select this model
                    } else {
                      // Start download
                      invoke("download_whisper_model", { modelName: m.id });
                    }
                  };

                  return (
                    <button
                      key={m.id}
                      className={`toggle-btn ${isSelected ? "active" : ""}`}
                      onClick={handleClick}
                      style={{
                        position: "relative",
                        overflow: "hidden",
                        opacity: isDownloading ? 0.85 : 1,
                        cursor: isDownloading ? "wait" : "pointer",
                        minHeight: 52,
                      }}
                    >
                      {/* Background progress fill */}
                      {isDownloading && (
                        <div style={{
                          position: "absolute",
                          left: 0, top: 0, bottom: 0,
                          width: `${st.progress}%`,
                          background: "linear-gradient(90deg, rgba(108,92,231,0.15), rgba(0,206,201,0.15))",
                          transition: "width 0.4s ease",
                          borderRadius: "inherit",
                          zIndex: 0,
                        }} />
                      )}

                      <div style={{ position: "relative", zIndex: 1, display: "flex", alignItems: "center", gap: 10, width: "100%" }}>
                        <div style={{ fontSize: 18, flexShrink: 0 }}>{m.icon}</div>
                        <div style={{ flex: 1, textAlign: "left" }}>
                          <div style={{ fontSize: 12, fontWeight: 700 }}>{m.name}</div>
                          <div style={{ fontSize: 10, opacity: 0.6 }}>{m.size} · {m.desc}</div>
                        </div>
                        <div style={{ flexShrink: 0, fontSize: 11, fontWeight: 700 }}>
                          {isDownloading ? (
                            <span style={{ color: "var(--vf-accent)" }}>
                              {st.progress}%
                            </span>
                          ) : isDownloaded ? (
                            <span style={{ color: "var(--vf-success)" }}>
                              {isSelected ? "● Selected" : "✓ Ready"}
                            </span>
                          ) : isError ? (
                            <span style={{ color: "var(--vf-danger)" }}>✕ Retry</span>
                          ) : (
                            <span style={{ color: "var(--vf-primary-light)" }}>
                              ↓ Download
                            </span>
                          )}
                        </div>
                      </div>
                    </button>
                  );
                })}
              </div>
            </div>
          );
        })()}
      </div>

      {/* ── Section 3: LLM Prompt Polish ── */}
      <div className="section">
        <div className="section-header">
          <span className="section-number">03</span>
          <span className="section-title">LLM Prompt Polish</span>
        </div>

        {/* LLM Enable/Disable toggle */}
        <div className="switch-row" style={{ marginBottom: 12 }}>
          <div>
            <div className="switch-row-label">Enable LLM Polish</div>
            <div className="switch-row-desc">Transform raw transcription with AI before pasting</div>
          </div>
          <label className="switch-control">
            <input
              type="checkbox"
              checked={settings.llm_enabled}
              onChange={(e) => update("llm_enabled", e.target.checked)}
            />
            <span className="switch-slider" />
          </label>
        </div>

        {/* Only show options when LLM is enabled */}
        {settings.llm_enabled && (
          <>
            {/* Model selector */}
            <div className="card" style={{ marginBottom: 10 }}>
              <div className="card-label">Groq Model</div>
              <div className="card-desc">Choose which LLM processes your text</div>
              <div className="toggle-group">
                <button
                  className={`toggle-btn ${settings.groq_model === "llama-3.1-8b-instant" ? "active" : ""}`}
                  onClick={() => update("groq_model", "llama-3.1-8b-instant")}
                >
                  <div className="toggle-btn-icon">⚡</div>
                  <div className="toggle-btn-name">Llama 3.1 8B</div>
                  <div className="toggle-btn-desc">Fast</div>
                </button>
                <button
                  className={`toggle-btn ${settings.groq_model === "meta-llama/llama-4-scout-17b-16e-instruct" ? "active" : ""}`}
                  onClick={() => update("groq_model", "meta-llama/llama-4-scout-17b-16e-instruct")}
                >
                  <div className="toggle-btn-icon">🧠</div>
                  <div className="toggle-btn-name">Llama 4 Scout</div>
                  <div className="toggle-btn-desc">Powerful</div>
                </button>
              </div>
            </div>

            {/* Prompt mode */}
            <div className="toggle-group">
              {[
                { key: "coding", icon: "💻", name: "Coding", desc: "Dev prompts" },
                { key: "email", icon: "📧", name: "Email", desc: "Professional" },
                { key: "general", icon: "✨", name: "General", desc: "Clean up" },
                { key: "custom", icon: "🔧", name: "Custom", desc: "Your prompt" },
              ].map((m) => (
                <button
                  key={m.key}
                  className={`toggle-btn ${settings.prompt_mode === m.key ? "active" : ""}`}
                  onClick={() => update("prompt_mode", m.key)}
                >
                  <div className="toggle-btn-icon">{m.icon}</div>
                  <div className="toggle-btn-name">{m.name}</div>
                </button>
              ))}
            </div>

            {settings.prompt_mode === "custom" && (
              <div className="card" style={{ marginTop: 10 }}>
                <div className="card-label">Custom System Prompt</div>
                <textarea
                  className="textarea-field"
                  placeholder="Define how the LLM should transform your dictation..."
                  value={settings.custom_prompt}
                  onChange={(e) => update("custom_prompt", e.target.value)}
                />
              </div>
            )}
          </>
        )}
      </div>

      {/* ── Section 4: Preferences ── */}
      <div className="section">
        <div className="section-header">
          <span className="section-number">04</span>
          <span className="section-title">Preferences</span>
        </div>

        <div className="switch-row">
          <div>
            <div className="switch-row-label">Auto-Paste</div>
            <div className="switch-row-desc">Automatically paste text at cursor position</div>
          </div>
          <label className="switch-control">
            <input
              type="checkbox"
              checked={settings.auto_paste}
              onChange={(e) => update("auto_paste", e.target.checked)}
            />
            <span className="switch-slider" />
          </label>
        </div>

        <div className="switch-row">
          <div>
            <div className="switch-row-label">Accessibility</div>
            <div className="switch-row-desc">Required for typing into other apps</div>
          </div>
          <span className={`access-badge ${accessGranted ? "granted" : "denied"}`}>
            {accessGranted ? "✓ Granted" : "Grant →"}
          </span>
        </div>
      </div>

      {/* ── Save Button ── */}
      <button
        className={`save-btn ${saved ? "saved" : ""}`}
        onClick={handleSave}
      >
        {saved ? "✓ Settings Saved" : "Save Settings"}
      </button>

      {/* ── Footer ── */}
      <footer className="app-footer">
        VoxForge v0.1.0 — Built with Tauri + Deepgram + Groq
      </footer>
    </div>
  );
}
