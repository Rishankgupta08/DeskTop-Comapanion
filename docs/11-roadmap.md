# OpenMate Development Roadmap

## Phase 1: Foundation & Core Architecture — COMPLETE
- **1-A**: Tech stack setup (Tauri v2 + React 19 + TypeScript + Vite + Tailwind CSS).
- **1-B**: Local SQLite database & memory engine (`tokio-rusqlite`).
- **1-C**: BYOK Keychain integration (`keyring`) & Google Gemini direct REST adapter.
- **1-D**: 4 Companion modes (`Play`, `Coder`, `Assistant`, `PersonalFriend`).
- **1-E**: Screen context awareness pipeline (`xcap` in-memory capture with guaranteed zeroization).
- **1-F**: Settings screen, permission engine, onboarding flow, and smoke testing.

---

## Phase 2: Action Engine, Voice & Proactive Assistance — COMPLETE
- **Voice Pipeline [DR-005]**:
  - In-memory microphone capture using `cpal` + `hound`.
  - Gemini multimodal Audio API for speech-to-text transcription.
  - Text-to-speech synthesis with background playback via `rodio`.
  - Full buffer zeroization upon discard (`AudioClip::discard`, `AudioOutput::discard`).
- **Tool Engine [DR-018]**:
  - Assistant Mode tools: `open_application`, `read_file`, `write_file`.
  - Coder Mode tools: `read_file`, `list_directory`, `search_in_file`.
  - Filesystem write safety cards in Chat UI (Allow once / Always allow session / Deny).
- **Proactive Behavior Model [DR-017]**:
  - User-configurable toggle: `Off` / `Subtle` (default) / `Active`.
  - Idle return detector (>5 min idle) with mode-specific greeting messages without screenshot capture.
- **Extended Triggers [DR-007]**:
  - Cross-platform in-memory clipboard monitoring (`arboard`) with zero-retention 64-bit hashing.
  - Screen awareness triggers (active window & application change).
- **UI & Avatar Polish [DR-016, DR-033]**:
  - 2D SVG sprite rendering for 6 distinct states (`idle`, `thinking`, `talking`, `happy`, `concerned`, `listening`).
  - Floating breathing animation, occasional eye blink, and listening audio pulse.
  - Markdown rendering (bold, inline code, code blocks, bullet points) in chat.
  - Message timestamps (HH:MM) and copy-to-clipboard on hover.
