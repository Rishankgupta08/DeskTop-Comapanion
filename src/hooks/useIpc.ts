/**
 * Tauri IPC bindings — typed wrappers for all backend commands.
 *
 * All backend calls go through this file. Components never import `@tauri-apps/api` directly.
 * This gives us a single place to mock for tests and to update if the IPC contract changes.
 *
 * Security note: the API key is passed TO the backend here but is NEVER stored
 * in component state or returned from any of these calls. [PP-004, DR-011]
 */

import { invoke } from "@tauri-apps/api/core";
import type {
  Capability,
  CompanionMode,
  ConversationMessage,
  MemoryEntry,
  PermissionState,
  PermissionStatus,
} from "../types";

// ── Centralized IPC Helper with Safe Error Normalization ──────────────────────

async function ipc<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (err: unknown) {
    if (typeof err === "string") {
      throw err;
    } else if (err && typeof err === "object" && "message" in err) {
      throw String((err as { message: unknown }).message);
    } else {
      throw "An unexpected communication error occurred with the desktop service.";
    }
  }
}

// ── Permission commands ───────────────────────────────────────────────────────

export async function getPermissions(): Promise<PermissionStatus[]> {
  try {
    const raw = await ipc<[Capability, PermissionState][]>("get_permissions");
    return raw.map(([capability, state]) => ({ capability, state }));
  } catch (err) {
    console.warn("Failed to load permissions:", err);
    return [
      { capability: "screen_capture", state: "off" },
      { capability: "microphone", state: "off" },
      { capability: "clipboard", state: "off" },
      { capability: "app_launch", state: "off" },
      { capability: "filesystem_read", state: "off" },
      { capability: "filesystem_write", state: "off" },
    ];
  }
}

export async function setPermission(
  capability: Capability,
  newState: PermissionState
): Promise<void> {
  return ipc<void>("set_permission", { capability, newState });
}

// ── API key commands ──────────────────────────────────────────────────────────

/** Returns true/false — never the key value. [PP-004] */
export async function hasApiKey(): Promise<boolean> {
  try {
    return await ipc<boolean>("has_api_key");
  } catch {
    return false;
  }
}

/**
 * Store the user's Gemini API key in the OS keychain.
 * The key string is never returned or stored in frontend state after this call.
 */
export async function setApiKey(key: string): Promise<void> {
  return ipc<void>("set_api_key", { key });
}

export async function deleteApiKey(): Promise<void> {
  return ipc<void>("delete_api_key");
}

export async function validateApiKey(): Promise<boolean> {
  try {
    return await ipc<boolean>("validate_api_key");
  } catch {
    return false;
  }
}

// ── Mode commands ─────────────────────────────────────────────────────────────

export async function getMode(): Promise<CompanionMode> {
  try {
    return await ipc<CompanionMode>("get_mode");
  } catch {
    return "assistant";
  }
}

export async function setMode(mode: CompanionMode): Promise<void> {
  return ipc<void>("set_mode", { mode });
}

// ── Memory commands ───────────────────────────────────────────────────────────

export async function getMemories(): Promise<MemoryEntry[]> {
  try {
    return await ipc<MemoryEntry[]>("get_memories");
  } catch (err) {
    console.warn("Failed to fetch memories:", err);
    return [];
  }
}

export async function saveMemory(
  content: string,
  tags: string[]
): Promise<MemoryEntry> {
  return ipc<MemoryEntry>("save_memory", { content, tags });
}

export async function deleteMemory(id: string): Promise<void> {
  return ipc<void>("delete_memory", { id });
}

export async function clearMemories(): Promise<number> {
  return ipc<number>("clear_memories");
}

// ── Conversation commands ─────────────────────────────────────────────────────

export async function getConversationHistory(
  limit?: number
): Promise<ConversationMessage[]> {
  try {
    return await ipc<ConversationMessage[]>("get_conversation_history", { limit });
  } catch {
    return [];
  }
}

export async function newSession(): Promise<string> {
  return ipc<string>("new_session");
}

// ── Chat commands ─────────────────────────────────────────────────────────────

export async function sendMessage(
  message: string,
  mode?: CompanionMode
): Promise<string> {
  return ipc<string>("send_message", { message, mode });
}

// ── Screen Context commands (Phase 1-E) ───────────────────────────────────────

export async function requestScreenContext(query?: string): Promise<string> {
  return ipc<string>("request_screen_context", { query });
}

export async function getContextStatus(): Promise<{
  permission_state: string;
  monitor_active: boolean;
}> {
  try {
    return await ipc<{ permission_state: string; monitor_active: boolean }>(
      "get_context_status"
    );
  } catch {
    return {
      permission_state: "off",
      monitor_active: false,
    };
  }
}

// ── Tool execution commands (Phase 2-A) ───────────────────────────────────────

export async function executeTool(
  toolName: string,
  args: Record<string, string>
): Promise<import("../types").ToolResult> {
  return ipc<import("../types").ToolResult>("execute_tool", {
    toolName,
    args,
  });
}

// ── Voice commands (Phase 2-B) ────────────────────────────────────────────────

export async function startVoiceInput(): Promise<import("../types").VoiceResult> {
  return ipc<import("../types").VoiceResult>("start_voice_input");
}

// ── Proactive commands (Phase 2-C) ────────────────────────────────────────────

export async function getProactiveMode(): Promise<import("../types").ProactiveMode> {
  try {
    const mode = await ipc<string>("get_proactive_mode");
    return (mode as import("../types").ProactiveMode) || "subtle";
  } catch {
    return "subtle";
  }
}

export async function setProactiveMode(mode: import("../types").ProactiveMode): Promise<void> {
  return ipc<void>("set_proactive_mode", { mode });
}

// ── Identity commands (Feature 1) ─────────────────────────────────────────────

export async function getCompanionName(): Promise<string> {
  try {
    return await ipc<string>("get_companion_name");
  } catch {
    return "OpenMate";
  }
}

export async function setCompanionName(name: string): Promise<void> {
  return ipc<void>("set_companion_name", { name });
}

export async function getUserName(): Promise<string> {
  try {
    return await ipc<string>("get_user_name");
  } catch {
    return "";
  }
}

export async function setUserName(name: string): Promise<void> {
  return ipc<void>("set_user_name", { name });
}

// ── Ambient commands (Feature 2) ──────────────────────────────────────────────

export async function generateAmbientMessage(): Promise<string> {
  return ipc<string>("generate_ambient_message");
}
