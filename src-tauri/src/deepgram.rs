//! VoxForge — Deepgram WebSocket real-time STT.
//!
//! Streams audio to Deepgram's WebSocket API and receives
//! word-by-word transcription results in real time.

use serde::Deserialize;
use std::io::{Read, Write};
use std::net::TcpStream;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

pub struct DeepgramClient {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
}

#[derive(Debug, Deserialize)]
pub struct DeepgramResponse {
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    pub channel: Option<Channel>,
    pub is_final: Option<bool>,
    pub speech_final: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct Channel {
    pub alternatives: Vec<Alternative>,
}

#[derive(Debug, Deserialize)]
pub struct Alternative {
    pub transcript: String,
    pub confidence: f64,
}

impl DeepgramClient {
    /// Connect to Deepgram WebSocket API with TLS.
    pub fn connect(api_key: &str) -> Result<Self, String> {
        let url_str = format!(
            "wss://api.deepgram.com/v1/listen?\
             model=nova-3&\
             language=en&\
             smart_format=true&\
             punctuate=true&\
             interim_results=true&\
             utterance_end_ms=1000&\
             vad_events=true&\
             encoding=linear16&\
             sample_rate=16000&\
             channels=1"
        );

        let request = tungstenite::http::Request::builder()
            .uri(&url_str)
            .header("Authorization", format!("Token {}", api_key))
            .header("Host", "api.deepgram.com")
            .header("Upgrade", "websocket")
            .header("Connection", "Upgrade")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tungstenite::handshake::client::generate_key(),
            )
            .body(())
            .map_err(|e| format!("Failed to build request: {}", e))?;

        let (ws, response) =
            tungstenite::connect(request).map_err(|e| format!("WebSocket connect failed: {}", e))?;

        eprintln!(
            "[VoxForge] Connected to Deepgram (status: {})",
            response.status()
        );

        // Set the underlying TCP stream to non-blocking for real-time reads
        match ws.get_ref() {
            MaybeTlsStream::NativeTls(tls_stream) => {
                let tcp = tls_stream.get_ref();
                tcp.set_nonblocking(true)
                    .map_err(|e| format!("Failed to set non-blocking: {}", e))?;
            }
            MaybeTlsStream::Plain(tcp) => {
                tcp.set_nonblocking(true)
                    .map_err(|e| format!("Failed to set non-blocking: {}", e))?;
            }
            _ => {
                eprintln!("[VoxForge] Warning: couldn't set non-blocking mode");
            }
        }

        Ok(Self { ws })
    }

    /// Send a chunk of f32 audio (converted to i16 PCM bytes).
    pub fn send_audio(&mut self, samples: &[f32]) -> Result<(), String> {
        // Convert f32 [-1.0, 1.0] to i16 PCM bytes (little-endian)
        let bytes: Vec<u8> = samples
            .iter()
            .flat_map(|&s| {
                let clamped = s.max(-1.0).min(1.0);
                let i16_val = (clamped * 32767.0) as i16;
                i16_val.to_le_bytes()
            })
            .collect();

        self.ws
            .send(Message::Binary(bytes.into()))
            .map_err(|e| format!("Send error: {}", e))
    }

    /// Try to receive a transcript result (non-blocking).
    /// Returns Ok(None) if no data is available yet.
    pub fn try_recv(&mut self) -> Result<Option<TranscriptResult>, String> {
        match self.ws.read() {
            Ok(Message::Text(text)) => {
                let text_str: &str = &text;

                // Parse the JSON response
                let resp: DeepgramResponse = match serde_json::from_str(text_str) {
                    Ok(r) => r,
                    Err(_) => return Ok(None), // Skip unparseable messages
                };

                if let Some(channel) = resp.channel {
                    if let Some(alt) = channel.alternatives.first() {
                        if !alt.transcript.is_empty() {
                            return Ok(Some(TranscriptResult {
                                text: alt.transcript.clone(),
                                is_final: resp.is_final.unwrap_or(false),
                                speech_final: resp.speech_final.unwrap_or(false),
                            }));
                        }
                    }
                }

                Ok(None)
            }
            Ok(Message::Close(_)) => {
                eprintln!("[VoxForge] Deepgram WebSocket closed by server");
                Err("Connection closed".to_string())
            }
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                // No data available yet — non-blocking, just return None
                Ok(None)
            }
            Err(e) => {
                let err_str = format!("{}", e);
                // Ignore WouldBlock-like errors
                if err_str.contains("WouldBlock") || err_str.contains("Resource temporarily unavailable") {
                    return Ok(None);
                }
                Err(format!("Recv error: {}", e))
            }
        }
    }

    /// Send close frame to signal end of audio.
    pub fn close(&mut self) -> Result<(), String> {
        // Set back to blocking for clean close
        match self.ws.get_ref() {
            MaybeTlsStream::NativeTls(tls_stream) => {
                let _ = tls_stream.get_ref().set_nonblocking(false);
            }
            MaybeTlsStream::Plain(tcp) => {
                let _ = tcp.set_nonblocking(false);
            }
            _ => {}
        }

        // Send close frame
        let _ = self.ws.close(Some(tungstenite::protocol::CloseFrame {
            code: tungstenite::protocol::frame::coding::CloseCode::Normal,
            reason: "Done".into(),
        }));

        eprintln!("[VoxForge] Deepgram connection closed");
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct TranscriptResult {
    pub text: String,
    pub is_final: bool,
    pub speech_final: bool,
}
