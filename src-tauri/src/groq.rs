//! VoxForge — Groq LLM prompt polishing.
//!
//! Sends transcribed text to Groq API for polishing
//! into structured, professional prompts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PromptMode {
    Direct,  // No polishing — raw transcription
    Coding,  // Polish into coding prompt
    Email,   // Polish into professional email
    General, // Clean grammar, punctuation
    Custom,  // User-defined system prompt
}

impl Default for PromptMode {
    fn default() -> Self {
        Self::Direct
    }
}

/// Available Groq models
pub const MODELS: &[(&str, &str)] = &[
    ("llama-3.1-8b-instant", "Llama 3.1 8B (Fast)"),
    ("meta-llama/llama-4-scout-17b-16e-instruct", "Llama 4 Scout 17B"),
];

#[derive(Serialize)]
struct GroqRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct GroqResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

fn system_prompt_for_mode(mode: &PromptMode, custom: &str) -> String {
    match mode {
        PromptMode::Direct => unreachable!(),
        PromptMode::Coding => {
            "You are a prompt engineer for coding tasks. Transform the user's rough voice \
             dictation into a clear, structured, actionable prompt for an AI coding assistant. \
             Include specific file/module names mentioned, technical requirements (types, error \
             handling, testing), clear action verbs, and production quality expectations. \
             Output ONLY the polished prompt, nothing else. Do not add markdown formatting."
                .to_string()
        }
        PromptMode::Email => {
            "You are a professional email writer. Transform the user's rough voice dictation \
             into a polished, professional email with proper greeting, clear structure, \
             appropriate tone, and sign-off. Output ONLY the email text, nothing else."
                .to_string()
        }
        PromptMode::General => {
            "Clean up the following voice dictation: fix grammar, add punctuation, improve \
             clarity while keeping the original meaning intact. Output ONLY the cleaned text."
                .to_string()
        }
        PromptMode::Custom => custom.to_string(),
    }
}

/// Polish text using Groq API.
/// Returns the polished text, or the original text on failure.
pub fn polish_prompt(
    api_key: &str,
    raw_text: &str,
    mode: &PromptMode,
    custom_prompt: &str,
    model: &str,
) -> String {
    if *mode == PromptMode::Direct || api_key.is_empty() {
        return raw_text.to_string();
    }

    let system = system_prompt_for_mode(mode, custom_prompt);

    // Use the selected model, fallback to llama-3.1-8b-instant
    let model_id = if model.is_empty() {
        "llama-3.1-8b-instant"
    } else {
        model
    };

    let request_body = GroqRequest {
        model: model_id.to_string(),
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: system,
            },
            ChatMessage {
                role: "user".to_string(),
                content: raw_text.to_string(),
            },
        ],
        temperature: 0.3,
        max_tokens: 1024,
    };

    eprintln!("[VoxForge] Groq request: model={}, mode={:?}", model_id, mode);

    let client = reqwest::blocking::Client::new();

    match client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
    {
        Ok(resp) => {
            if resp.status().is_success() {
                match resp.json::<GroqResponse>() {
                    Ok(data) => {
                        if let Some(choice) = data.choices.first() {
                            let polished = choice.message.content.trim().to_string();
                            eprintln!("[VoxForge] Groq polished: \"{}\" → \"{}\"", raw_text, polished);
                            return polished;
                        }
                    }
                    Err(e) => eprintln!("[VoxForge] Groq parse error: {}", e),
                }
            } else {
                let status = resp.status();
                let body = resp.text().unwrap_or_default();
                eprintln!("[VoxForge] Groq API error {}: {}", status, body);
            }
        }
        Err(e) => eprintln!("[VoxForge] Groq request failed: {}", e),
    }

    // Fallback to raw text
    raw_text.to_string()
}
