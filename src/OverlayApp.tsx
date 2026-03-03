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
            handler((e as unknown as { payload: T }).payload)
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

// Read the current outer (physical) position of the overlay window and return
// the logical coordinates, accounting for the device pixel ratio / scale factor.
async function readLogicalPosition(): Promise<{ x: number; y: number } | undefined> {
    if (!isTauri()) return undefined;
    try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const win = getCurrentWindow();
        const [phys, scale] = await Promise.all([
            win.outerPosition(),
            win.scaleFactor(),
        ]);
        return { x: phys.x / scale, y: phys.y / scale };
    } catch {
        return undefined;
    }
}

// ─── Types ───────────────────────────────────────────────────────────────────
type BubbleState = "idle" | "recording" | "processing" | "done" | "error";

// ─── Internal Waveform Circle Component ─────────────────────────────────────
// 7 vertical bars drawn inside the circle bubble. Bar heights are driven by
// `audioLevel` (0–1). The cosine envelope makes the center bar tallest and
// the outer bars shortest, giving an equalizer appearance.

const BAR_COUNT = 7;

const BAR_META = Array.from({ length: BAR_COUNT }, (_, i) => {
    const norm = (i - 3) / 3; // -1 … +1, centre = 0
    const envelope = Math.cos((norm * Math.PI) / 2); // 1 at centre, 0 at edges
    return {
        envelope,
        delay: (i * 0.07).toFixed(2),
        dur: (0.45 + Math.abs(norm) * 0.25).toFixed(2),
    };
});

function WaveformCircle({ audioLevel }: { audioLevel: number }) {
    return (
        <div className="waveform-circle-inner">
            {BAR_META.map((m, i) => (
                <div
                    key={i}
                    className="waveform-inner-bar"
                    style={{
                        height: `${Math.max(4, 4 + m.envelope * 24 * (0.15 + audioLevel * 0.85))}px`,
                        ["--bar-delay" as string]: `${m.delay}s`,
                        ["--bar-dur" as string]: `${m.dur}s`,
                    } as React.CSSProperties}
                />
            ))}
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

// ─── Audio Level Hook ────────────────────────────────────────────────────────

function useAudioLevel(recording: boolean): number {
    const [level, setLevel] = useState(0);
    const ref = useRef<ReturnType<typeof setInterval> | null>(null);

    useEffect(() => {
        if (recording) {
            ref.current = setInterval(async () => {
                const l = await safeInvoke<number>("get_audio_level");
                if (l !== undefined) setLevel(l);
            }, 50); // ~20 fps for smooth waveform
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
    const hideTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
    const audioLevel = useAudioLevel(state === "recording");
    const isDragging = useRef(false);

    // Stable BAR_META reference — useMemo prevents regeneration on each render
    const _barMeta = useMemo(() => BAR_META, []);
    void _barMeta; // consumed by WaveformCircle via module-level constant

    // ── Ensure the window is the correct size on mount ──
    useEffect(() => {
        if (!isTauri()) return;
        (async () => {
            try {
                const { getCurrentWindow } = await import("@tauri-apps/api/window");
                const { LogicalSize } = await import("@tauri-apps/api/dpi");
                await getCurrentWindow().setSize(new LogicalSize(88, 88));
            } catch { }
        })();
    }, []);

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
            }, delay);
        } else {
            setState("idle");
        }
    }, []);

    // Save the window's current logical position to persistent settings.
    // Called 80ms after a drag ends so Tauri has reported the final position.
    const saveCurrentPosition = useCallback(async () => {
        const pos = await readLogicalPosition();
        if (pos) {
            await safeInvoke("save_window_position", { x: pos.x, y: pos.y });
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

            if (isDragging.current) {
                // Persist the new position after the OS finishes moving the window.
                setTimeout(() => { saveCurrentPosition(); }, 80);
            } else {
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

    // ── State-derived class ──
    const stateClass =
        state === "recording"
            ? "is-recording"
            : state === "processing"
                ? "is-processing"
                : state === "done"
                    ? "is-done"
                    : "";

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

    // ── Bubble (idle / recording / processing / done) ──
    return (
        <div className="voice-bubble-container">
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

                    {/* Recording: internal waveform bars */}
                    {state === "recording" && (
                        <WaveformCircle audioLevel={audioLevel} />
                    )}

                    {/* Processing: animated bouncing dots */}
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

                {/* Blinking red dot — top-right of bubble, visible while recording */}
                {state === "recording" && (
                    <div className="bubble-rec-dot" />
                )}
            </div>
        </div>
    );
}
