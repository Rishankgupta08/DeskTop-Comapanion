# OpenMate Decision Register

This register documents all architectural, security, and design decisions for OpenMate.

---

## Confirmed Decisions

| ID | Topic | Decision | Status | Implementation Notes |
|---|---|---|---|---|
| **DR-001** | Project Name & Purpose | OpenMate — Open-source AI desktop companion | CONFIRMED | Standalone desktop assistant |
| **DR-002** | License | MIT License | CONFIRMED | Open source permissive |
| **DR-003** | Core Tech Stack | Tauri v2.x (Rust backend) + React 19 / TypeScript / Vite | CONFIRMED | Cross-platform architecture |
| **DR-004** | Gemini model | gemini-2.0-flash / gemini-3.5-flash | CONFIRMED | REST v1beta API direct BYOK |
| **DR-005** | Voice provider | Gemini Audio API for STT, Gemini/system TTS for speech output | CONFIRMED | Implemented in `platform::audio` via `cpal` + `hound` + `rodio`, gated by `Capability::Microphone` |
| **DR-006** | Local Database | SQLite via `tokio-rusqlite` / `rusqlite` | CONFIRMED | Encrypted local storage |
| **DR-007** | Extended screen triggers | Clipboard change detection & idle-return greeting | CONFIRMED | `platform::clipboard` hash-only monitoring; mode-specific return greetings without screenshots |
| **DR-008** | Screenshot Persistence | Strictly in-memory `Vec<u8>` buffers; zero disk writes, zero temp files, explicit zeroization on discard | CONFIRMED | Guaranteed in `context::handle_context_request` |
| **DR-011** | Key Storage | OS native credential store (macOS Keychain / Windows Credential Manager via `keyring`) | CONFIRMED | Implemented in `platform::keychain` with dev fallback |
| **DR-012** | Permission Model | Explicit permissions defaulting to `Off`; require `PermissionToken` compile-time proof | CONFIRMED | Implemented in `engine::permission` |
| **DR-015** | Companion Modes | Play, Coder, Assistant, Personal Friend | CONFIRMED | Distinct system prompts in `engine::mode` |
| **DR-016** | Avatar Visuals | 2D SVG sprite rendering with state animations | CONFIRMED | Idle, thinking, talking, happy, concerned, listening |
| **DR-017** | Proactive behavior | User-configurable Off/Subtle/Active toggle, default Subtle | CONFIRMED | `ContextEngine::should_trigger_proactive` with idle duration evaluation |
| **DR-018** | Tool schema | Assistant: open_application, read_file, write_file. Coder: read_file, list_directory, search_in_file | CONFIRMED | Implemented in `engine::tool`, filesystem writes gated by interactive confirmation |
| **DR-020** | Prompt Injection Defense | Mandatory `UNTRUSTED_CONTEXT_NOTICE` prepended to all desktop context prompts | CONFIRMED | Prepended in `ai::gemini` and `engine::context` |
| **DR-027** | Architecture Model | BYOK (Bring Your Own Key); direct client-to-Google Gemini REST API calls; no hosted proxy | CONFIRMED | Direct client architecture |
| **DR-028** | Telemetry & Privacy | Zero telemetry, zero external tracking, local logs only | CONFIRMED | Complete local privacy |
| **DR-029** | Styling & Animation | Tailwind CSS + Framer Motion (no external UI component libraries) | CONFIRMED | Sleek dark UI design |
| **DR-030** | Screen Awareness Scope | Explicit user query, Active window change, Active application change | CONFIRMED | Implemented in `engine::context` |
| **DR-032** | Avatar Presentation | Draggable 2D desktop overlay with dynamic animated expressions | CONFIRMED | Floating breathing animation and drag-and-drop |
| **DR-033** | Chat UI | Slide-in bottom-right panel with auto-scroll and mode switcher | CONFIRMED | Markdown, timestamps, copy button |
| **DR-035** | Screenshot Capture Library | `xcap` crate for cross-platform in-memory screen capture | CONFIRMED | In-memory frame capture |
| **DR-036** | Avatar package format | PNG spritesheets + JSON manifest | CONFIRMED | Folder structure: `avatars/<name>/manifest.json` + `idle.png` + `thinking.png` + `talking.png` + `happy.png` + `concerned.png` + `listening.png` |
| **DR-037** | Plugin scope | Option C — Avatars + TypeScript modes + signed Rust tool plugins | CONFIRMED | Signing/trust model to be designed in Phase 3-C before any third-party execution is allowed |
| **DR-039** | Plugin sandboxing | Out-of-process execution via JSON-RPC 2.0 over stdin/stdout pipes. macOS: sandbox-exec. Windows: Job Objects. No shared address space with host. [OD-01 confirmed] | CONFIRMED | Documented in `docs/plugin-trust-model.md` |
| **DR-040** | Plugin cryptography | Ed25519 signatures via ed25519-dalek crate. Author signs plugin.toml + binary. OpenMate verifies at load time. [OD-02 confirmed] | CONFIRMED | Documented in `docs/plugin-trust-model.md` |
| **DR-041** | Trust registry | Bundled trusted_authors.json + optional online sync with offline fallback. Users can manually approve author keys. [OD-03 confirmed] | CONFIRMED | Documented in `docs/plugin-trust-model.md` |
| **DR-042** | Plugin author tooling | openmate-cli subcommand: openmate-cli plugin sign, openmate-cli plugin verify, openmate-cli plugin package [OD-04 confirmed] | CONFIRMED | Documented in `docs/plugin-trust-model.md` |
| **DR-043** | Unsigned plugins | Blocked by default. Developer Mode opt-in in Settings shows persistent warning banner. Every tool call requires explicit interactive confirmation in Developer Mode. [OD-05 confirmed] | CONFIRMED | Documented in `docs/plugin-trust-model.md` |
| **DR-044** | Plugin binary format | Multi-architecture binaries in bin/ folder: plugin-darwin-arm64, plugin-darwin-x86_64, plugin-windows-x86_64.exe [OD-06 confirmed] | CONFIRMED | Documented in `docs/plugin-trust-model.md` |

---

## TBD / Deferred Decisions

| ID | Topic | Status | Notes |
|---|---|---|---|
| **DR-009** | Screenshot Compression Tuning | TBD | Defaulting to JPEG quality 85 |
| **DR-019** | Adaptive History Token Window | TBD | Defaulting to 20 conversation turns |
| **DR-038** | CI/CD setup | TBD | Deferred — not part of Phase 3 scope |
