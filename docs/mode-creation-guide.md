# Creating an OpenMate Mode Extension

OpenMate mode extensions allow you to create custom companion personalities, study buddies, gaming guides, or productivity partners without modifying the OpenMate core codebase [DR-037].

Mode extensions are sandboxed TypeScript modules that run in OpenMate's client environment and define system prompts, ambient greetings, and optional tag parsers.

---

## 1. What a Mode Extension Can Do

- **Custom System Prompt**: Define rich companion personalities, tone of voice, formatting rules, and context-aware behavior via `buildSystemPrompt(context)`.
- **Ambient Message Pools**: Supply time-of-day specific proactive thoughts and check-ins (morning, afternoon, evening) when the user is idle.
- **Custom Tool Tag Handlers**: Optionally interpret domain-specific tags within assistant responses through `handleToolTag(tag, args)`.
- **First-Class Mode Selector Integration**: Automatically appear in the ChatPanel mode switcher with custom icons and descriptions.

---

## 2. Security Boundaries & Constraints

To protect users against malicious scripts and untrusted execution, mode extensions operate under strict security boundaries:

| Capability | Allowed? | Notes |
| :--- | :--- | :--- |
| Custom System Prompts | ✅ Yes | Must return pure strings. |
| Custom Ambient Pools | ✅ Yes | Structured strings by time of day. |
| Execute Native Binaries | ❌ No | Arbitrary native binary execution is forbidden. |
| Direct Tauri IPC Calls | ❌ No | Extensions cannot invoke Tauri backend IPC commands directly. |
| Filesystem Access | ❌ Sandboxed | All filesystem actions must route through OpenMate's `PermissionEngine` and `ToolEngine`. |
| Network Requests | ❌ Restricted | Extensions cannot establish unauthorized network sockets or bypass user consent. |

---

## 3. Directory Structure

Each mode extension lives in its own subdirectory inside the `modes/` folder:

```text
modes/
  └── <mode-id>/
      ├── manifest.json
      └── index.ts
```

### `manifest.json` Schema

```json
{
  "name": "Study Buddy",
  "id": "study-buddy",
  "version": "1.0.0",
  "author": "OpenMate Team",
  "description": "Helps you focus and study",
  "icon": "📚",
  "openmate_version": ">=0.1.0"
}
```

#### Field Specifications
- **`name`** *(string, required)*: Human-readable display name shown in the UI.
- **`id`** *(string, required)*: Unique slug matching `/^[a-z0-9-]{2,32}$/` (lowercase alphanumeric and hyphens only; no path traversals).
- **`version`** *(string, required)*: Semantic version string (e.g. `"1.0.0"`).
- **`author`** *(string, required)*: Creator name, organization, or GitHub handle.
- **`description`** *(string, required)*: Concise 1-sentence summary of the companion's mode.
- **`icon`** *(string, required)*: Single emoji or character (e.g. `"📚"`, `"🎮"`, `"🧙"`).
- **`openmate_version`** *(string, required)*: Minimum supported OpenMate version range (e.g. `">=0.1.0"`).

---

## 4. Writing `index.ts`

Your `index.ts` file must default-export an object implementing the `ModeExtension` interface:

```typescript
import type { ModeExtension, ModeContext } from "../../src/types/mode-extension";

const StudyBuddy: ModeExtension = {
  manifest: {
    name: "Study Buddy",
    id: "study-buddy",
    version: "1.0.0",
    author: "OpenMate Team",
    description: "Helps you focus and study",
    icon: "📚",
    openmate_version: ">=0.1.0",
  },

  buildSystemPrompt(context: ModeContext): string {
    return `You are ${context.companionName}, a focused study companion for ${context.userName || "the user"}.
Help them understand concepts clearly.
Use encouraging, patient language.
Break complex topics into step-by-step explanations.
Celebrate when they understand something! *adjusts glasses*`;
  },

  ambientMessages: {
    morning: [
      "Ready to learn something amazing today?",
      "Grab a warm drink and let's review your goals!"
    ],
    afternoon: [
      "How's the studying going? Need a tricky concept explained?",
      "Time for a quick 2-minute stretch break!"
    ],
    evening: [
      "Great study session today! Your brain needs rest to consolidate memory.",
      "Wrap up for the night when you're ready — rest is part of learning."
    ],
  },
};

export default StudyBuddy;
```

---

## 5. Mode Context Reference

When `buildSystemPrompt(context)` is called before an AI request, OpenMate passes the following runtime context:

```typescript
export interface ModeContext {
  companionName: string;   // Active companion name (e.g. "OpenMate" or user-configured name)
  userName: string;        // User's preferred name or handle
  memories: string[];      // Relevant persistent long-term memories
  currentTime: string;     // Local formatted time string
}
```

---

## 6. Testing & Verifying Your Mode

1. Place your folder in `modes/<mode-id>/`.
2. Run `npx tsc --noEmit` to verify type checking.
3. Start OpenMate in dev mode:
   ```bash
   npm run tauri dev
   ```
4. Open the Companion Chat Panel (`Space` or click Avatar).
5. Locate your mode in the mode selector bar (or dropdown if >6 modes are loaded).
6. Chat with the companion to verify its customized personality and response style.

---

## 7. Publishing & Distribution

*(Community mode registry and verified package repository are scheduled for Phase 3-C / Phase 4)*.

Currently, community members can share mode extension folders directly on GitHub or as `.zip` bundles to be unpacked into `modes/`.
