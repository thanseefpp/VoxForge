import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import "./overlay.css";

// ─── Tauri Helpers ───────────────────────────────────────────────────────────
const isTauri = (): boolean =>
    typeof window !== "undefined" &&
    ("__TAURI__" in window || "__TAURI_INTERNALS__" in window);

type UnlistenFn = () => void;

async function safeListen<T>(
    event: string,
    handler: (payload: T) => void
): Promise<UnlistenFn> {
    if (!isTauri()) return () => { };
    try {
        const { listen } = await import("@tauri-apps/api/event");
        return await listen<{ payload: T }>(event, (e) =>
            handler((e as any).payload)
        );
    } catch {
        return () => { };
    }
}

async function safeInvoke<T>(
    cmd: string,
    args?: Record<string, unknown>
): Promise<T | undefined> {
    if (!isTauri()) return undefined;
    try {
        const { invoke } = await import("@tauri-apps/api/core");
        return await invoke<T>(cmd, args);
    } catch {
        return undefined;
    }
}

async function startDrag() {
    if (!isTauri()) return;
    try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        await getCurrentWindow().startDragging();
    } catch { }
}

// ─── Click-through management ────────────────────────────────────────────────
// The overlay window is transparent and always-on-top. We need to make it
// click-through (ignore cursor events) EXCEPT when the mouse is over the bubble.

async function setIgnoreCursor(ignore: boolean) {
    if (!isTauri()) return;
    try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        await getCurrentWindow().setIgnoreCursorEvents(ignore);
    } catch { }
}

// ─── Types ───────────────────────────────────────────────────────────────────
type BubbleState = "idle" | "recording" | "processing" | "done" | "error";

// ─── Waveform Component ─────────────────────────────────────────────────────

function WaveformBars({ audioLevel }: { audioLevel: number }) {
    const barCount = 28;

    // Use useMemo so random durations don't regenerate each render
    const barMeta = useMemo(
        () =>
            Array.from({ length: barCount }, (_, i) => ({
                delay: (i * 0.04).toFixed(2),
                dur: (0.4 + Math.random() * 0.5).toFixed(2),
                centerDist: Math.abs(i - barCount / 2) / (barCount / 2),
            })),
        []
    );

    const bars = barMeta.map((meta, i) => {
        const baseHeight = 10 + (1 - meta.centerDist) * 30;
        const dynamicHeight = baseHeight * (0.25 + audioLevel * 0.75);

        return (
            <div
                key={i}
                className="wave-bar"
                style={{
                    height: `${Math.max(3, dynamicHeight)}px`,
                    ["--wave-delay" as any]: `${meta.delay}s`,
                    ["--wave-dur" as any]: `${meta.dur}s`,
                }}
            />
        );
    });

    const left = bars.slice(0, Math.floor(barCount / 2));
    const right = bars.slice(Math.floor(barCount / 2));

    return (
        <div className="waveform-container">
            {left}
            <div className="wave-stop-btn" title="Click to stop" />
            {right}
        </div>
    );
}

// ─── Icons ───────────────────────────────────────────────────────────────────

function MicIcon() {
    return (
        <svg
            width="26"
            height="26"
            viewBox="0 0 24 24"
            fill="none"
            stroke="#A29BFE"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
        >
            <rect x="9" y="2" width="6" height="12" rx="3" />
            <path d="M5 10a7 7 0 0 0 14 0" />
            <line x1="12" y1="18" x2="12" y2="22" />
            <line x1="8" y1="22" x2="16" y2="22" />
        </svg>
    );
}

function ProcessingDots() {
    return (
        <div className="bubble-dots">
            <div className="proc-dot" style={{ animationDelay: "0s" }} />
            <div className="proc-dot" style={{ animationDelay: "0.2s" }} />
            <div className="proc-dot" style={{ animationDelay: "0.4s" }} />
        </div>
    );
}

function CheckIcon() {
    return (
        <svg
            width="26"
            height="26"
            viewBox="0 0 24 24"
            fill="none"
            stroke="#00B894"
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeLinejoin="round"
        >
            <polyline points="20 6 9 17 4 12" />
        </svg>
    );
}

// ─── Timer Hook ──────────────────────────────────────────────────────────────

function useTimer(running: boolean): string {
    const [seconds, setSeconds] = useState(0);
    const ref = useRef<ReturnType<typeof setInterval> | null>(null);

    useEffect(() => {
        if (running) {
            setSeconds(0);
            ref.current = setInterval(() => setSeconds((s) => s + 1), 1000);
        } else {
            if (ref.current) clearInterval(ref.current);
        }
        return () => {
            if (ref.current) clearInterval(ref.current);
        };
    }, [running]);

    const m = String(Math.floor(seconds / 60)).padStart(2, "0");
    const s = String(seconds % 60).padStart(2, "0");
    return `${m}:${s}`;
}

// ─── Audio Level Hook ────────────────────────────────────────────────────────

function useAudioLevel(recording: boolean): number {
    const [level, setLevel] = useState(0);
    const ref = useRef<ReturnType<typeof setInterval> | null>(null);

    useEffect(() => {
        if (recording) {
            ref.current = setInterval(async () => {
                const l = await safeInvoke<number>("get_audio_level");
                if (l !== undefined) setLevel(l);
            }, 50); // ~20fps for smooth waveform
        } else {
            if (ref.current) clearInterval(ref.current);
            setLevel(0);
        }
        return () => {
            if (ref.current) clearInterval(ref.current);
        };
    }, [recording]);

    return level;
}

// ─── OverlayApp ──────────────────────────────────────────────────────────────

export default function OverlayApp() {
    const [state, setState] = useState<BubbleState>("idle");
    const [errorMsg, setErrorMsg] = useState("An error occurred.");
    const [streamingText, setStreamingText] = useState("");
    const hideTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
    const timer = useTimer(state === "recording");
    const audioLevel = useAudioLevel(state === "recording");
    const isDragging = useRef(false);

    // ── Start with cursor events ignored (click-through) ──
    useEffect(() => {
        setIgnoreCursor(true);
    }, []);

    const show = useCallback((s: BubbleState) => {
        if (hideTimer.current) clearTimeout(hideTimer.current);
        setState(s);
    }, []);

    const hide = useCallback((delay = 0) => {
        if (delay > 0) {
            hideTimer.current = setTimeout(() => {
                setState("idle");
                setStreamingText("");
            }, delay);
        } else {
            setState("idle");
            setStreamingText("");
        }
    }, []);

    // Listen for backend events
    useEffect(() => {
        const unsubs: UnlistenFn[] = [];

        (async () => {
            if (!isTauri()) {
                setTimeout(() => show("recording"), 500);
                return;
            }

            unsubs.push(
                await safeListen<string>("pipeline-status", (payload) => {
                    switch (payload) {
                        case "recording":
                            setStreamingText("");
                            show("recording");
                            break;
                        case "transcribing":
                        case "processing":
                        case "polishing":
                        case "pasting":
                            show("processing");
                            break;
                        case "done":
                            show("done");
                            hide(1400);
                            break;
                    }
                })
            );

            unsubs.push(
                await safeListen<string>("pipeline-error", (payload) => {
                    setErrorMsg(payload || "Recording failed.");
                    show("error");
                })
            );

            unsubs.push(
                await safeListen<string>("streaming-text", (text) => {
                    setStreamingText(text);
                })
            );
        })();

        return () => unsubs.forEach((fn) => fn());
    }, [show, hide]);

    // ── Mouse enters bubble area → allow clicks ──
    const handleMouseEnter = () => {
        setIgnoreCursor(false);
    };

    // ── Mouse leaves bubble area → ignore clicks (click-through) ──
    const handleMouseLeave = () => {
        setIgnoreCursor(true);
    };

    // ── Click vs Drag ──
    const handleMouseDown = (e: React.MouseEvent) => {
        isDragging.current = false;
        const startX = e.clientX;
        const startY = e.clientY;

        const handleMouseMove = (ev: MouseEvent) => {
            if (
                Math.abs(ev.clientX - startX) > 5 ||
                Math.abs(ev.clientY - startY) > 5
            ) {
                isDragging.current = true;
                startDrag();
                document.removeEventListener("mousemove", handleMouseMove);
            }
        };

        const handleMouseUp = () => {
            document.removeEventListener("mousemove", handleMouseMove);
            document.removeEventListener("mouseup", handleMouseUp);
            if (!isDragging.current) {
                handleBubbleClick();
            }
        };

        document.addEventListener("mousemove", handleMouseMove);
        document.addEventListener("mouseup", handleMouseUp);
    };

    const handleBubbleClick = async () => {
        if (state === "idle" || state === "recording") {
            await safeInvoke("toggle_recording");
        }
    };

    const handleTryAgain = async () => {
        show("idle");
        await safeInvoke("toggle_recording");
    };

    const handleDismiss = () => {
        show("idle");
    };

    // ── Error Card ──
    if (state === "error") {
        return (
            <div className="voice-bubble-container bubble-in">
                <div
                    className="bubble-error-card"
                    onMouseEnter={handleMouseEnter}
                    onMouseLeave={handleMouseLeave}
                >
                    <div className="bubble-error-accent" />
                    <div className="bubble-error-body">
                        <h2 className="bubble-error-title">Recording Error</h2>
                        <p className="bubble-error-msg">{errorMsg}</p>
                        <div className="bubble-error-actions">
                            <button className="btn-retry" onClick={handleTryAgain}>
                                Retry
                            </button>
                            <button className="btn-dismiss" onClick={handleDismiss}>
                                Dismiss
                            </button>
                        </div>
                    </div>
                </div>
            </div>
        );
    }

    // ── Bubble ──
    const stateClass =
        state === "recording"
            ? "is-recording"
            : state === "processing"
                ? "is-processing"
                : state === "done"
                    ? "is-done"
                    : "";

    return (
        <div className="voice-bubble-container">
            {/* Interactive area — enables cursor events on hover */}
            <div
                className="bubble-hit-area"
                onMouseEnter={handleMouseEnter}
                onMouseLeave={handleMouseLeave}
            >
                <div
                    className={`voice-bubble ${stateClass} bubble-in`}
                    onMouseDown={handleMouseDown}
                    title={
                        state === "idle"
                            ? "Click to start recording"
                            : state === "recording"
                                ? "Click to stop"
                                : ""
                    }
                >
                    {/* Idle: mic icon */}
                    {state === "idle" && (
                        <div className="bubble-icon">
                            <MicIcon />
                        </div>
                    )}

                    {/* Recording: waveform with center stop button */}
                    {state === "recording" && (
                        <WaveformBars audioLevel={audioLevel} />
                    )}

                    {/* Processing: animated dots */}
                    {state === "processing" && (
                        <div className="bubble-icon">
                            <ProcessingDots />
                        </div>
                    )}

                    {/* Done: checkmark */}
                    {state === "done" && (
                        <div className="bubble-icon bubble-check">
                            <CheckIcon />
                        </div>
                    )}
                </div>

                {/* Timer badge */}
                {state === "recording" && (
                    <div className="bubble-timer-badge">{timer}</div>
                )}
            </div>

            {/* Streaming text preview */}
            {streamingText && (state === "recording" || state === "processing") && (
                <div
                    className="bubble-preview"
                    onMouseEnter={handleMouseEnter}
                    onMouseLeave={handleMouseLeave}
                >
                    {streamingText.length > 80
                        ? "…" + streamingText.slice(-80)
                        : streamingText}
                </div>
            )}
        </div>
    );
}
