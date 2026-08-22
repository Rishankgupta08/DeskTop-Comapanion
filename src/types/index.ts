/**
 * OpenMate TypeScript type definitions
 *
 * These types mirror the Rust DTOs in src-tauri/src/.
 * Any change to a Rust struct or enum MUST be reflected here.
 */

// ── Permission types ─────────────────────────────────────────────────────────

export type Capability =
  | "screen_capture"
  | "microphone"
  | "filesystem_read"
  | "filesystem_write"
  | "app_launch"
  | "clipboard";

export type PermissionState = "off" | "ask" | "allow";

export interface PermissionStatus {
  capability: Capability;
  state: PermissionState;
}

// ── Companion modes ───────────────────────────────────────────────────────────

export type BuiltinCompanionMode = "play" | "coder" | "assistant" | "personal_friend";
export type CompanionMode = BuiltinCompanionMode | (string & {});

export const MODE_LABELS: Record<BuiltinCompanionMode, string> = {
  play: "Play",
  coder: "Coder",
  assistant: "Assistant",
  personal_friend: "Personal Friend",
};

export * from "./mode-extension";

// ── Memory types ──────────────────────────────────────────────────────────────

export interface MemoryEntry {
  id: string;
  content: string;
  tags: string[];
  created_at: string;
  updated_at: string;
}

// ── Conversation types ────────────────────────────────────────────────────────

export type MessageRole = "user" | "assistant" | "system";

export interface ConversationMessage {
  id: string;
  session_id: string;
  role: MessageRole;
  content: string;
  created_at: string;
}

/** A transient message in the chat UI (not yet persisted). */
export interface ChatMessage {
  id: string;
  role: MessageRole;
  content: string;
  timestamp: number;
  isLoading?: boolean;
}

// ── Error types ───────────────────────────────────────────────────────────────

export type OpenMateErrorKind =
  | "PermissionDenied"
  | "PermissionNotFound"
  | "KeychainError"
  | "NoApiKey"
  | "ProviderError"
  | "ProviderUnreachable"
  | "InvalidApiKey"
  | "DatabaseError"
  | "MemoryNotFound"
  | "ToolError"
  | "InvalidToolArguments"
  | "CaptureError"
  | "Internal"
  | "SerializationError";

export interface OpenMateError {
  kind: OpenMateErrorKind;
  message: string;
}

// ── Settings types ────────────────────────────────────────────────────────────

export interface AppSettings {
  /** Whether the user has completed onboarding. */
  onboardingComplete: boolean;
  /** The currently selected companion mode. */
  mode: CompanionMode;
}

// ── Avatar types (ADR-003, DR-036) ────────────────────────────────────────────

export type AvatarState =
  | "idle"
  | "talking"
  | "thinking"
  | "listening"
  | "happy"
  | "concerned";

export interface AvatarManifest {
  name: string;
  version: string;
  author: string;
  description: string;
  type: string;
  states: AvatarState[];
  openmate_version: string;
}

export interface AvatarInfo {
  name: string;
  author: string;
  description: string;
  is_active: boolean;
}


// ── Context types (Phase 1-E) ─────────────────────────────────────────────────

export interface ContextStatus {
  permission_state: string;
  monitor_active: boolean;
}

// ── Tool types (Phase 2-A) ───────────────────────────────────────────────────

export interface ToolResult {
  tool: string;
  success: boolean;
  output: string;
  error?: string | null;
}

// ── Voice types (Phase 2-B) ──────────────────────────────────────────────────

export interface VoiceResult {
  transcription: string;
  response: string;
}

// ── Proactive types (Phase 2-C) ──────────────────────────────────────────────

export type ProactiveMode = "off" | "subtle" | "active";

// ── Plugin types (Phase 3-D) [DR-039 through DR-044] ──────────────────────────

export type PluginTrustLevel =
  | "builtin"
  | "community"
  | "user_approved"
  | "unknown"
  | "revoked";

export interface PluginToolInfo {
  name: string;
  description: string;
}

export interface PluginInfo {
  id: string;
  name: string;
  version: string;
  author: string;
  author_pubkey: string;
  description: string;
  trust_level: PluginTrustLevel;
  required_capabilities: string[];
  tools: PluginToolInfo[];
}
