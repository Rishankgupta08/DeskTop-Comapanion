# Creating an OpenMate Native Rust Plugin

OpenMate native plugins allow developers and community contributors to extend OpenMate with custom native tools, API integrations, and local automations while maintaining maximum security and privacy [DR-039 through DR-044].

---

## 1. Plugin Architecture Overview

OpenMate native plugins run as **sandboxed child processes** that communicate with OpenMate over standard input/output (`stdin`/`stdout`) using **JSON-RPC 2.0**.

```
┌────────────────────────────────────────────────────────┐
│                   OPENMATE HOST (Rust)                 │
│  • Manages SQLite DB, Keyring, PermissionEngine        │
│  • Enforces 10-second timeouts                         │
│  • Gating: Only allows approved capabilities           │
└──────────────────────────┬─────────────────────────────┘
                           │ JSON-RPC 2.0 (stdin / stdout)
┌──────────────────────────▼─────────────────────────────┐
│              SANDBOXED PLUGIN PROCESS                  │
│  • Isolated memory address space                       │
│  • Environment stripped via env_clear()                │
│  • Restricted file/network permissions                 │
└────────────────────────────────────────────────────────┘
```

---

## 2. Directory Structure

Each plugin lives inside its own folder in `plugins/<plugin-id>/`:

```text
plugins/
  └── <plugin-id>/
      ├── plugin.toml             # Manifest & capability declarations
      ├── plugin.sig              # Detached Ed25519 cryptographic signature
      ├── bin/
      │   ├── plugin-darwin-arm64     # macOS Apple Silicon
      │   ├── plugin-darwin-x86_64    # macOS Intel
      │   └── plugin-windows-x86_64.exe # Windows x64
      └── README.md
```

---

## 3. Manifest Specification (`plugin.toml`)

```toml
[plugin]
id = "weather-companion"
name = "Weather Companion"
version = "1.0.0"
author = "OpenMate Team"
author_pubkey = "ed25519:3b6f2c7d9e1a4f5b8c0d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b"
description = "Fetches local weather forecasts and severe weather alerts"
openmate_version = ">=0.1.0"

[capabilities]
required = ["network_access"]
optional = []

[entrypoint]
macos_arm64 = "bin/plugin-darwin-arm64"
macos_x86_64 = "bin/plugin-darwin-x86_64"
windows_x86_64 = "bin/plugin-windows-x86_64.exe"

[[tools]]
name = "get_weather"
description = "Get current weather for a city"
parameters = { type = "object", properties = { city = { type = "string" } }, required = ["city"] }
```

### Approved Capabilities
Plugins can only declare capabilities from the approved whitelist:
- `network_access`: External HTTP/network requests
- `filesystem_read`: Reading local files
- `filesystem_write`: Writing local files
- `screen_capture`: Screen context analysis
- `microphone`: Audio input
- `clipboard`: Clipboard change detection
- `app_launch`: Launching external applications

---

## 4. Authoring with `openmate-cli`

OpenMate provides the official `openmate-cli` command-line utility for author key generation, signing, verification, and packaging.

### Step 1: Generate an Author Keypair
```bash
openmate-cli plugin keygen
```
*Outputs `private.key` (keep secret!) and `public.key` (`ed25519:<hex>`).*

### Step 2: Build Your Binary
Build your Rust plugin binary for your target architecture:
```bash
cargo build --release
mkdir -p bin/
cp target/release/my-plugin bin/plugin-darwin-arm64
```

### Step 3: Sign Your Plugin
```bash
openmate-cli plugin sign --key private.key --plugin .
```
*Computes the SHA-256 hash of `plugin.toml` + binary and generates `plugin.sig`.*

### Step 4: Verify Your Plugin
```bash
openmate-cli plugin verify --plugin .
# Output: Signature valid.
```

### Step 5: Package as `.omp`
```bash
openmate-cli plugin package --plugin . --output my-plugin.omp
```

---

## 5. JSON-RPC 2.0 Protocol

Your plugin binary must loop on `stdin` lines and write JSON-RPC 2.0 responses to `stdout`.

### Incoming Request:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tool_call",
  "params": {
    "tool": "get_weather",
    "arguments": { "city": "Delhi" }
  }
}
```

### Outgoing Success Response:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "output": "Currently 32°C, sunny",
    "success": true
  }
}
```

### Outgoing Error Response:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32001,
    "message": "Failed to connect to weather API"
  }
}
```

---

## 6. Testing in Developer Mode

1. Copy your plugin folder into the `plugins/` directory.
2. In OpenMate: **Settings → General → Enable Developer Mode**.
3. In **Settings → Plugins**, locate your plugin and enable it.
4. Interact with OpenMate via chat or voice to test tool invocations.
