# Contributing to VoxForge

Thank you for your interest in contributing to VoxForge! 🎉

## 🚀 Getting Started

### Prerequisites

- **Rust** (latest stable) — [rustup.rs](https://rustup.rs/)
- **Node.js** v18+ — [nodejs.org](https://nodejs.org/)
- **macOS** (currently macOS-only due to native APIs)
- Xcode Command Line Tools: `xcode-select --install`

### Development Setup

```bash
# Clone the repo
git clone https://github.com/thanseefpp/VoxForge.git
cd VoxForge

# Install Node dependencies
npm install

# Run in development mode
npm run tauri dev
```

### API Keys (Optional for Development)

- **Deepgram** — Free tier at [deepgram.com](https://deepgram.com/) (for real-time STT)
- **Groq** — Free tier at [groq.com](https://groq.com/) (for LLM polishing)
- **Whisper** — No API key needed (runs locally)

## 📝 How to Contribute

### Reporting Bugs

1. Check existing [issues](https://github.com/thanseefpp/VoxForge/issues) first
2. Open a new issue with:
   - Steps to reproduce
   - Expected vs actual behavior
   - macOS version + hardware (Intel/Apple Silicon)
   - Console logs (`npm run tauri dev` terminal output)

### Suggesting Features

Open a [feature request](https://github.com/thanseefpp/VoxForge/issues/new) with:
- Clear description of the feature
- Why it would be useful
- Any relevant examples or mockups

### Submitting Pull Requests

1. Fork the repository
2. Create a feature branch: `git checkout -b feat/my-feature`
3. Make your changes
4. Test thoroughly with `npm run tauri dev`
5. Commit with conventional commits:
   - `feat: add new feature`
   - `fix: resolve bug`
   - `docs: update documentation`
   - `style: formatting changes`
   - `refactor: code restructuring`
6. Push and open a PR against `main`

## 🏗️ Project Structure

```
VoxForge/
├── src/                  # React frontend (TypeScript)
│   ├── App.tsx          # Settings page
│   ├── OverlayApp.tsx   # Floating bubble overlay
│   └── *.css            # Styles
├── src-tauri/src/       # Rust backend
│   ├── lib.rs           # Main app logic + Tauri commands
│   ├── audio.rs         # Audio capture (cpal)
│   ├── deepgram.rs      # Deepgram WebSocket STT
│   ├── whisper.rs       # Whisper offline STT
│   ├── groq.rs          # Groq LLM polishing
│   ├── focus.rs         # macOS focus tracking
│   └── paste.rs         # Clipboard + paste simulation
└── ...
```

## 🎨 Code Style

- **Rust**: Follow `rustfmt` defaults. Run `cargo fmt` before committing.
- **TypeScript/React**: Use consistent patterns with existing code.
- **CSS**: Use the design token system (`--vf-*` CSS variables).

## 💡 Good First Issues

Look for issues labeled [`good first issue`](https://github.com/thanseefpp/VoxForge/labels/good%20first%20issue) to get started!

## 📜 License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
