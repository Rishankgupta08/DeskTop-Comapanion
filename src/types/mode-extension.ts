/**
 * OpenMate TypeScript Mode Extension Interface
 *
 * Defines the contract for external community-contributed companion modes. [DR-037]
 * Mode extensions run in the sandboxed React/Vite client context and are loaded
 * dynamically via the ModeLoader.
 */

export interface ModeManifest {
  name: string;           // Display name (e.g. "Study Buddy")
  id: string;             // Unique slug, alphanumeric + hyphens (/^[a-z0-9-]{2,32}$/)
  version: string;        // SemVer format (e.g. "1.0.0")
  author: string;         // Creator name or handle
  description: string;    // Shown in mode selector and chat header
  icon: string;           // Emoji or single character (e.g. "📚")
  openmate_version: string; // OpenMate compatibility range
}

export interface ModeContext {
  companionName: string;
  userName: string;
  memories: string[];
  currentTime: string;
}

export interface ToolResponse {
  output: string;
  success: boolean;
}

export interface AmbientMessagePool {
  morning: string[];
  afternoon: string[];
  evening: string[];
}

export interface ModeExtension {
  // Metadata
  manifest: ModeManifest;

  // System prompt builder
  // Called before every Gemini request in this mode
  buildSystemPrompt(context: ModeContext): string;

  // Optional: handle tool tags in Gemini response
  // Return null to let OpenMate handle it
  handleToolTag?(tag: string, args: string): Promise<ToolResponse | null>;

  // Optional: ambient message pool
  ambientMessages?: AmbientMessagePool;
}
