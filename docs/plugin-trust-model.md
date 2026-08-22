# OpenMate Rust Plugin Signing & Trust Model

| Document ID | Status | Phase | Target Release | Owner Review Required |
| :--- | :--- | :--- | :--- | :--- |
| **ADR-004 / RFC-004** | **PROPOSED** | Phase 3-C | Post-v0.2.0 | **Yes (Pending Confirmation)** |

---

## Executive Summary

As confirmed in [DR-037](file:///Users/mac/Documents/Companion/docs/DECISION-REGISTER.md), OpenMate supports a modular extension architecture comprising:
1. **Avatars**: PNG asset packages + JSON manifest ([DR-036](file:///Users/mac/Documents/Companion/docs/DECISION-REGISTER.md), Phase 3-A).
2. **Modes**: Sandboxed TypeScript modules ([DR-037](file:///Users/mac/Documents/Companion/docs/DECISION-REGISTER.md), Phase 3-B).
3. **Rust Tool Plugins**: High-performance native tools executing external logic (Phase 3-C).

Because native Rust code compiles to machine instructions that could potentially access system resources, read memory, or execute unauthorized operations, **no third-party native plugin may be executed without cryptographic verification, process-level sandboxing, and strict integration with OpenMate's `PermissionEngine`**.

This document proposes the **Rust Plugin Signing, Sandboxing, and Trust Model** for OpenMate.

---

## 1. Signing Mechanism

### 1.1 Evaluated Options

| Criteria | Option A: Ed25519 Keypairs | Option B: OS Code Signing Certificates | Option C: Checksum Hash Registry |
| :--- | :--- | :--- | :--- |
| **Description** | Authors generate an Ed25519 keypair, sign the package payload, and OpenMate verifies against pinned/registered public keys. | Requires authors to obtain Apple Developer ID certificates (macOS) and Microsoft Authenticode certificates (Windows). | OpenMate maintains a centralized list of approved SHA-256 hashes of plugin archives. |
| **Barrier to Entry** | **Zero cost**. Authors generate keys in seconds via `openmate-cli` or `ssh-keygen`. | **High cost** ($99/year Apple Developer fee, $300+/year Windows EV certificate). Prohibitive for indie/open-source contributors. | **Low cost**, but requires central infrastructure to maintain and distribute the hash list. |
| **Cryptographic Strength** | High (256-bit security level, standard in modern cryptographic systems). | High (X.509 PKI, backed by platform root CAs). | Medium (Cryptographic hash integrity only; no author identity non-repudiation). |
| **Offline Verification** | Fully offline verification supported via embedded public keys or trust store. | Requires online OCSP/timestamping or platform CA chains. | Requires network connectivity to fetch updated hash registry or offline fallback. |
| **Revocation Model** | Revocation of compromised public keys via OpenMate trust registry updates. | Revocation via Apple/Microsoft CRL/OCSP. | Remove hash from central registry. |

### 1.2 Proposed Recommendation: **Option A (Ed25519 Keypairs with Hybrid Trust Store)**

> **Recommendation**: OpenMate will use **Ed25519 cryptographic signatures** generated using `ed25519-dalek` / `ring`. 
>
> - Every plugin package includes a `plugin.sig` containing the Ed25519 signature of the manifest (`plugin.toml`) and binary executable.
> - The author's public key is declared in `plugin.toml` and verified against OpenMate's embedded **Trusted Community Registry** or user-approved local keyrings.

#### Rationale:
1. **Permissive Open-Source Accessibility**: Does not impose financial or corporate barriers on open-source developers.
2. **Speed & Security**: Ed25519 signature verification takes $< 50\,\mu\text{s}$ in Rust and provides collision and side-channel resistance.
3. **Deterministic Verification**: Signatures verify the exact binary and manifest combination, preventing post-signing tampering or capability escalation.

---

## 2. Trust Levels

OpenMate defines three explicit trust tiers:

```
┌────────────────────────────────────────────────────────────────────────┐
│                        TRUST TIER HIERARCHY                            │
├────────────────────────────────────────────────────────────────────────┤
│                                                                        │
│   [ LEVEL 1: Built-In Core ]                                          │
│   • Shipped with OpenMate core binary                                  │
│   • Direct in-process execution                                        │
│   • Full access to internal engines                                    │
│                                                                        │
│   [ LEVEL 2: Community Signed ]                                        │
│   • Valid Ed25519 signature from verified author in Trust Registry     │
│   • Out-of-process sandbox execution                                   │
│   • Granular permissions mediated by PermissionEngine                  │
│                                                                        │
│   [ LEVEL 3: Unknown / Unsigned ]                                      │
│   • Unsigned, self-signed without trust anchor, or invalid signature   │
│   • BLOCKED BY DEFAULT                                                 │
│   • Requires explicit Developer Mode override + audible warning        │
│                                                                        │
└────────────────────────────────────────────────────────────────────────┘
```

### 2.1 Level 1 — OpenMate Built-In (Fully Trusted)
- **Scope**: Native core tools (`read_file`, `write_file`, `open_application`, `list_directory`).
- **Execution**: Runs directly in the OpenMate host process.
- **Trust Anchor**: Compiled into the signed OpenMate application binary.

### 2.2 Level 2 — Community Signed (Verified Signature)
- **Scope**: Verified third-party plugins distributed through the community repository.
- **Execution**: Spawned as isolated child processes under OS sandboxing.
- **Trust Anchor**: Public key matches an entry in OpenMate's bundled `trusted_authors.json` or verified registry.
- **Permission Enforcement**: Requires explicit user consent (`PermissionStatus::Ask` or `PermissionStatus::Allow`) before any system resource is accessed.

### 2.3 Level 3 — Unknown / Unsigned (Blocked by Default)
- **Scope**: Development builds, experimental tools, or unrecognized third-party binaries.
- **Execution**: **Refused by default**.
- **Developer Mode Exception**: Can only run if the user enables **"Developer Mode & Unsigned Plugins"** in Settings. When enabled, a persistent warning banner appears, and every tool call requires explicit interactive confirmation.

---

## 3. Capability Boundaries & Sandboxing

### 3.1 Capability Matrix

| System Resource | Level 1 (Built-In) | Level 2 (Community Signed) | Level 3 (Developer Mode) |
| :--- | :--- | :--- | :--- |
| **Host Process Memory** | Shared | **Isolated** (Separate Process) | **Isolated** (Separate Process) |
| **API Keys & Keyring** | Direct access | **Strictly Forbidden** (No Keyring Access) | **Strictly Forbidden** |
| **Screen Buffer / Vision** | Direct in-memory | **Mediated** (Requires `Capability::ScreenCapture`) | **Mediated** (Interactive Prompts) |
| **Microphone / Audio** | Direct stream | **Mediated** (Requires `Capability::Microphone`) | **Mediated** (Interactive Prompts) |
| **Filesystem Access** | PermissionEngine gated | **Restricted to plugin workspace** + explicit permission | Restricted to plugin workspace |
| **Network Sockets** | Direct HTTPS client | **Mediated** (`Capability::NetworkAccess`) | **Mediated** (Interactive Prompts) |
| **Custom Capabilities** | Core defined | **Declarative** (Registered in Settings UI) | **Declarative** |

### 3.2 PermissionEngine Integration
- Plugins **cannot** bypass OpenMate's compile-time `PermissionToken` model ([DR-012](file:///Users/mac/Documents/Companion/docs/DECISION-REGISTER.md)).
- When a plugin wishes to perform an action (e.g. read a file or call an external API), it sends a structured JSON-RPC request to OpenMate's plugin host.
- OpenMate validates the plugin's requested capability against `PermissionEngine`. If the permission is `Off`, the host rejects the request immediately. If `Ask`, an interactive user prompt is rendered.

```
Plugin (Child Process)              OpenMate Host (Rust Core)              User UI
       │                                       │                              │
       │─── JSON-RPC: tool_call(args) ────────>│                              │
       │                                       │─── Check PermissionEngine ──>│
       │                                       │    (e.g., State == Ask)      │
       │                                       │<── User Approves ────────────│
       │                                       │                              │
       │                                       │─── Execute & Validate Token  │
       │<── JSON-RPC: tool_result(data) ───────│                              │
```

---

## 4. Sandboxing Architecture

### 4.1 Evaluated Approaches

| Criteria | Option A: Dynamic Libraries (`.dylib` / `.dll`) via `libloading` | Option B: Out-of-Process Binary via IPC (JSON-RPC) | Option C: WebAssembly Runtime (`wasmtime` / `extism`) |
| :--- | :--- | :--- | :--- |
| **Security Boundary** | ❌ **None**. Shared address space. A crash or vulnerability in the plugin crashes or compromises OpenMate. | ✅ **OS Process Isolation**. Host memory, API keys, and credentials remain inaccessible to the plugin. | ✅ **Software Fault Isolation**. Memory is sandboxed inside the WASM linear memory model. |
| **Crash Resilience** | ❌ Segfault in `.dylib` terminates the entire OpenMate app. | ✅ Plugin crash terminates only child process; host logs error and continues. | ✅ WASM trap handled cleanly by runtime. |
| **Native Performance** | Instant in-memory function calls. | High ($< 0.5\,\text{ms}$ latency over anonymous pipes or Unix sockets). | Near-native CPU performance; overhead on host-to-WASM memory copies. |
| **OS Compatibility** | High maintenance (macOS code signing restrictions, Windows DLL loading quirks). | ✅ Standard OS process execution across macOS and Windows. | ✅ Single portable `.wasm` binary across all OS platforms. |
| **Hardware & Native Access** | Full native access. | Full native access (subject to OS sandbox rules). | Restricted to WASI capabilities. |

### 4.2 Proposed Recommendation: **Option B (Out-of-Process IPC with Native Executables)**

> **Recommendation**: OpenMate will implement **Option B: Out-of-Process Execution via Asynchronous IPC Pipes**.
>
> - Plugins compile to native platform executables (`plugin-darwin-arm64`, `plugin-windows-x86_64.exe`).
> - The OpenMate host spawns the plugin as a low-privilege child process communicating over `stdin`/`stdout` using JSON-RPC 2.0.
> - On macOS, child processes are wrapped with `sandbox-exec` profiles. On Windows, processes run in a restricted Job Object with disabled token privileges.

---

## 5. Plugin Package Format

Plugins are distributed as folders or `.omp` (OpenMate Plugin) zip archives:

```text
plugins/
  └── <plugin-id>/
      ├── plugin.toml             # Manifest & capability declarations
      ├── plugin.sig              # Detached Ed25519 signature
      ├── bin/
      │   ├── plugin-darwin-arm64
      │   ├── plugin-darwin-x86_64
      │   └── plugin-windows-x86_64.exe
      └── README.md
```

### 5.1 `plugin.toml` Schema

```toml
[plugin]
id = "weather-companion"
name = "Weather Companion"
version = "1.0.0"
author = "OpenMate Team"
author_pubkey = "ed25519:3b6f2c7d9e1a4f5b8c0d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b"
description = "Fetches local weather forecasts and severe weather alerts"
openmate_version = ">=0.2.0"

[capabilities]
required = ["network_access"]
optional = ["filesystem_read"]

[entrypoint]
macos_arm64 = "bin/plugin-darwin-arm64"
macos_x86_64 = "bin/plugin-darwin-x86_64"
windows_x86_64 = "bin/plugin-windows-x86_64.exe"

[[tools]]
name = "get_current_weather"
description = "Fetch current temperature and conditions for a given city"
parameters = { type = "object", properties = { city = { type = "string" } }, required = ["city"] }
```

---

## 6. Trust Verification Flow

When OpenMate boots or a plugin is activated, the following verification pipeline is executed:

```mermaid
flowchart TD
    A[Discover Plugin in plugins/ directory] --> B[Sanitize Plugin ID /^[a-z0-9-]{2,32}$/]
    B -->|Invalid Slug| X[REJECT: Path Traversal/Injection Risk]
    B -->|Valid Slug| C[Parse & Validate plugin.toml]
    C -->|Malformed Schema| Y[REJECT: Invalid Manifest]
    C -->|Valid Schema| D[Compute SHA-256 Hash of Binary Payload]
    D --> E[Verify Ed25519 Signature against author_pubkey via plugin.sig]
    E -->|Signature Mismatch| Z[REJECT: Cryptographic Verification Failed]
    E -->|Signature Valid| F{Check author_pubkey in Trust Registry}
    F -->|In Trusted Registry| G[Assign Trust Level 2: Community Signed]
    F -->|Not in Registry| H{Is Developer Mode Enabled?}
    H -->|No| W[BLOCK: Untrusted Author Key]
    H -->|Yes| I[Assign Trust Level 3: Unsigned/Developer]
    G --> J[Register Capabilities in PermissionEngine]
    I --> J
    J --> K[Spawn Sandboxed Child Process & Handshake]
```

### Step-by-Step Verification Sequence

1. **Path & Slug Validation**: Ensure the plugin folder and manifest `id` match `/^[a-z0-9-]{2,32}$/`.
2. **Payload Hash Check**: Compute SHA-256 over `plugin.toml` + platform binary.
3. **Cryptographic Signature Verification**: Verify `plugin.sig` against the author's public key using Ed25519.
4. **Trust Resolution**:
   - Check if `author_pubkey` is pinned in `trusted_authors.json`.
   - If not found, check if user has manually trusted this author key.
   - If untrusted and Developer Mode is disabled, fail with `PluginTrustError::UntrustedAuthor`.
5. **Capability Registration**: Register declared tools into OpenMate's tool catalog with associated permissions.
6. **Subprocess Spawning**: Launch native executable with restricted permissions, connected via standard I/O pipes.

---

## 7. Risks and Mitigations

| # | Threat Vector | Impact | Architectural Mitigation |
| :- | :--- | :--- | :--- |
| **1** | **Author Key Compromise / Supply Chain Attack** | Malicious update pushed under valid signature. | Hybrid Trust Store with centralized key revocation list (`revoked_keys.json`). Manifest includes capability declarations; any update demanding new capabilities requires re-authorization. |
| **2** | **Memory Corruption / Native Crash in Plugin** | Crash of OpenMate host or heap exploitation. | **Out-of-process execution (Option B)**. If the child process crashes, the host survives, logs the event, and marks the plugin as degraded. |
| **3** | **PermissionEngine Bypass** | Plugin attempts to perform raw file/socket I/O without user consent. | OS-level sandboxing (macOS `sandbox-exec`, Windows Job Objects) strips direct OS filesystem/network rights; all requests must route through host IPC. |
| **4** | **Credential & Data Exfiltration** | Plugin reads OpenMate SQLite DB, keychain, or memory. | Child process has no read access to OpenMate's Application Support directory or macOS Keychain. Environment variables containing API keys are explicitly stripped when spawning child processes. |
| **5** | **Resource Exhaustion / Denial of Service** | Plugin enters infinite loop, spawns fork bombs, or leaks memory. | Host process monitors CPU and memory thresholds; enforces strict request timeouts (default $10\,\text{s}$) with automated process termination on timeout. |

---

## 8. Open Decisions (For Owner Confirmation)

Before Phase 3-C implementation code is authored, the project owner should review and confirm the following design decisions:

| ID | Decision Item | Proposed Default | Alternative Options | Owner Status |
| :--- | :--- | :--- | :--- | :--- |
| **OD-01** | Sandboxing Runtime | **Option B: Out-of-Process Process Isolation via JSON-RPC** | Option C: WASM Runtime (`wasmtime`) | `PROPOSED` |
| **OD-02** | Cryptographic Algorithm | **Ed25519 (`ed25519-dalek`)** | ECDSA (P-256) / RSA-4096 | `PROPOSED` |
| **OD-03** | Trust Registry Model | **Bundled `trusted_authors.json` + User Whitelist** | Dynamic online sync registry | `PROPOSED` |
| **OD-04** | Author Tooling | **`openmate-cli plugin sign` subcommand** | Standalone authoring web tool | `PROPOSED` |
| **OD-05** | Unsigned Plugins (Dev Mode) | **Blocked by default; opt-in Developer Mode with UI banner** | Hard block with zero override | `PROPOSED` |
| **OD-06** | Binary Packaging | **Multi-architecture binaries in `bin/` folder** | Single architecture per archive | `PROPOSED` |

---

## 9. Conclusion & Next Steps

This document establishes a security-first, accessible foundation for OpenMate's native plugin ecosystem.

**Next Steps**:
1. Review open decisions (OD-01 through OD-06) with project owner.
2. Confirm architectural choices in [docs/DECISION-REGISTER.md](file:///Users/mac/Documents/Companion/docs/DECISION-REGISTER.md).
3. Proceed to Phase 3-C implementation upon approval.
