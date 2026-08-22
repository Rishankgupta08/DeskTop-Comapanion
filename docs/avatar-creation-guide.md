# Creating an OpenMate Avatar Package

OpenMate supports community-created 2D avatar packages [DR-036]. This guide explains how to design, package, and test your own custom avatar companion.

---

## Folder Structure

Place your avatar package directory inside the `avatars/` folder located in your OpenMate directory:

```text
avatars/
  <avatar-name>/
    manifest.json
    idle.png
    thinking.png
    talking.png
    happy.png
    concerned.png
    listening.png
```

> [!NOTE]
> The `<avatar-name>` folder name must exactly match the `"name"` field in your `manifest.json` and must contain only alphanumeric characters and hyphens (e.g., `cyber-cat`, `pixel-bot`, `mecha-fox`).

---

## `manifest.json` Specification

Every avatar package must include a valid `manifest.json` file in its root:

```json
{
  "name": "cyber-cat",
  "version": "1.0.0",
  "author": "Your Name",
  "description": "A sleek neon cyberpunk cat companion for your desktop.",
  "type": "avatar",
  "states": [
    "idle",
    "thinking",
    "talking",
    "happy",
    "concerned",
    "listening"
  ],
  "openmate_version": ">=0.1.0"
}
```

### Manifest Fields Reference

| Field | Type | Description | Example |
|---|---|---|---|
| `name` | string | Unique package identifier (alphanumeric + hyphens) | `"cyber-cat"` |
| `version` | string | Package version conforming to SemVer format | `"1.0.0"` |
| `author` | string | Creator name or handle | `"Jane Doe"` |
| `description` | string | Short description displayed in OpenMate Settings | `"A friendly companion"` |
| `type` | string | Must be `"avatar"` | `"avatar"` |
| `states` | array of strings | Must contain all 6 states (`idle`, `thinking`, `talking`, `happy`, `concerned`, `listening`) | `["idle", ...]` |
| `openmate_version` | string | OpenMate version compatibility specifier | `">=0.1.0"` |

> [!IMPORTANT]
> To ensure security and prevent arbitrary code execution, OpenMate strictly rejects manifests containing unexpected or unknown fields.

---

## Image Requirements

- **File Format**: PNG (`.png`) with alpha channel transparency.
- **Dimensions**: Recommended `200x200px` (square aspect ratio; scaled smoothly to desktop overlay dimensions).
- **Background**: Transparent (no solid background rectangle).
- **Visuals**: Each expression should be visually distinct to reflect the companion's live emotional and operational state.

---

## States Reference

| State Image | When Triggered | Expression Guidance |
|---|---|---|
| `idle.png` | Default resting state when waiting for user interaction | Calm, friendly, resting expression |
| `thinking.png` | LLM or tool processing in progress | Looking up, concentrating, or thought bubbles |
| `talking.png` | Companion is responding in chat or reading TTS | Open mouth, energetic, speaking expression |
| `happy.png` | Positive interaction, successful task, or celebration | Broad smile, joyful eyes (^ ^), sparkles |
| `concerned.png` | Error encountered, warning, or difficult query | Tilted brow, sympathetic or worried expression |
| `listening.png` | User is actively speaking during microphone input | Attentive eyes, sound-wave or listening indicator |

---

## Testing Your Avatar

1. Place your avatar folder into the `avatars/` directory in OpenMate:
   ```bash
   cp -r my-avatar /path/to/openmate/avatars/
   ```
2. Open OpenMate and go to **Settings** → **Avatar**.
3. Your avatar will appear in the list with its name, author, and description.
4. Click **Use this avatar** to immediately activate it on your desktop.
