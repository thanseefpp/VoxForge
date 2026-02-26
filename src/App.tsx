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
};

export default function App() {
  const [settings, setSettings] = useState<Settings>(defaultSettings);
  const [saved, setSaved] = useState(false);
  const [accessGranted, setAccessGranted] = useState(false);

  // Load settings on mount
  useEffect(() => {
    (async () => {
      const s = await invoke<Settings>("get_settings");
      if (s) setSettings(s);
      const a = await invoke<boolean>("check_accessibility");
      if (a !== undefined) setAccessGranted(a);
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
            <div className="toggle-btn-desc">Real-time streaming</div>
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
