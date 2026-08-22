/**
 * OpenMate Settings Screen
 *
 * Full-screen settings overlay featuring API key lifecycle management,
 * granular permission controls, avatar preview, memory inspection, and app metadata.
 * [SRS FR-027, FR-030, FR-031, FR-037, FR-038, FR-039]
 */

import { useState, useEffect } from "react";
import { motion } from "framer-motion";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  hasApiKey,
  setApiKey,
  deleteApiKey,
  getPermissions,
  setPermission,
  getMemories,
  deleteMemory,
  clearMemories,
  newSession,
  getProactiveMode,
  setProactiveMode,
  getCompanionName,
  setCompanionName,
  getUserName,
  setUserName,
  listAvatars,
  setActiveAvatar,
  getDeveloperMode,
  setDeveloperMode,
  listPlugins,
  approvePluginKey,
  removePlugin,
} from "../../hooks/useIpc";
import type {
  AvatarInfo,
  Capability,
  PermissionState,
  PermissionStatus,
  MemoryEntry,
  ProactiveMode,
  PluginInfo,
} from "../../types";
import MemoryPanel from "../memory/MemoryPanel";

interface SettingsProps {
  isOpen: boolean;
  onClose: () => void;
  onKeySaved?: () => void;
  onKeyRemoved?: () => void;
}

export default function Settings({
  isOpen,
  onClose,
  onKeySaved,
  onKeyRemoved,
}: SettingsProps) {
  // Companion Identity State [Feature 1.1]
  const [companionName, setCompanionNameState] = useState("OpenMate");
  const [userName, setUserNameState] = useState("");
  const [isSavingIdentity, setIsSavingIdentity] = useState(false);
  const [identitySavedMsg, setIdentitySavedMsg] = useState(false);

  // 4.1 API Key State
  const [apiKeySet, setApiKeySet] = useState<boolean | null>(null);
  const [isReplacingKey, setIsReplacingKey] = useState(false);
  const [newKeyInput, setNewKeyInput] = useState("");
  const [isSavingKey, setIsSavingKey] = useState(false);
  const [keyError, setKeyError] = useState<string | null>(null);
  const [confirmDeleteKey, setConfirmDeleteKey] = useState(false);

  // 4.2 Permissions State
  const [permissions, setPermissions] = useState<PermissionStatus[]>([]);

  // 4.3 Proactive Assistance State [DR-017]
  const [proactiveMode, setProactiveModeState] = useState<ProactiveMode>("subtle");

  // 4.4 Avatar State [Phase 3-A]
  const [avatars, setAvatars] = useState<AvatarInfo[]>([]);
  const [activatingAvatar, setActivatingAvatar] = useState<string | null>(null);

  // 4.5 Plugin State [Phase 3-D]
  const [plugins, setPlugins] = useState<PluginInfo[]>([]);
  const [developerMode, setDeveloperModeState] = useState(false);
  const [showDevModeModal, setShowDevModeModal] = useState(false);
  const [enabledPlugins, setEnabledPlugins] = useState<Record<string, boolean>>({});

  // 4.6 Memory State
  const [memories, setMemories] = useState<MemoryEntry[]>([]);
  const [isMemoryExpanded, setIsMemoryExpanded] = useState(false);
  const [confirmClearHistory, setConfirmClearHistory] = useState(false);
  const [historyClearedMsg, setHistoryClearedMsg] = useState(false);

  const loadData = async () => {
    try {
      const cName = await getCompanionName();
      setCompanionNameState(cName);

      const uName = await getUserName();
      setUserNameState(uName);

      const keyExists = await hasApiKey();
      setApiKeySet(keyExists);

      const perms = await getPermissions();
      setPermissions(perms);

      const pMode = await getProactiveMode();
      setProactiveModeState(pMode);

      const avatarList = await listAvatars();
      setAvatars(avatarList);

      const devMode = await getDeveloperMode();
      setDeveloperModeState(devMode);

      const pluginList = await listPlugins();
      setPlugins(pluginList);

      const mems = await getMemories();
      setMemories(mems);
    } catch {
      // Non-blocking load error handling
    }
  };

  useEffect(() => {
    if (isOpen) {
      loadData();
    }
  }, [isOpen]);

  // Handle API Key Replace
  const handleSaveNewKey = async () => {
    const trimmed = newKeyInput.trim();
    if (!trimmed) {
      setKeyError("API key cannot be empty.");
      setNewKeyInput("");
      return;
    }

    setIsSavingKey(true);
    setKeyError(null);

    try {
      await setApiKey(trimmed);
      // SECURITY: Clear key state immediately after IPC call completes
      setNewKeyInput("");
      setIsReplacingKey(false);
      setApiKeySet(true);
      onKeySaved?.();
    } catch (err) {
      setNewKeyInput("");
      setKeyError("Failed to save key: " + String(err));
    } finally {
      setIsSavingKey(false);
    }
  };

  // Handle API Key Delete
  const handleDeleteKey = async () => {
    try {
      await deleteApiKey();
      setApiKeySet(false);
      setConfirmDeleteKey(false);
      onKeyRemoved?.();
    } catch {
      // Delete error handling
    }
  };

  // Handle Permission Change with robust upsert & safe rollback
  const handleTogglePermission = async (cap: Capability, current: PermissionState) => {
    const nextState: PermissionState = current === "allow" ? "off" : "allow";
    const hadExistingEntry = permissions.some((p) => p.capability === cap);

    // Optimistic upsert update
    setPermissions((prev) => {
      const exists = prev.some((p) => p.capability === cap);
      if (exists) {
        return prev.map((p) => (p.capability === cap ? { ...p, state: nextState } : p));
      }
      return [...prev, { capability: cap, state: nextState }];
    });

    try {
      await setPermission(cap, nextState);
    } catch {
      // Revert safely on failure: restore previous state if entry existed, else remove optimistic entry
      setPermissions((prev) => {
        if (hadExistingEntry) {
          return prev.map((p) => (p.capability === cap ? { ...p, state: current } : p));
        }
        return prev.filter((p) => p.capability !== cap);
      });
    }
  };

  // Handle Memory Delete
  const handleDeleteMemory = async (id: string) => {
    const previous = [...memories];
    setMemories((prev) => prev.filter((m) => m.id !== id));

    try {
      await deleteMemory(id);
    } catch {
      setMemories(previous);
    }
  };

  // Handle Clear Memories
  const handleClearAllMemories = async () => {
    const previous = [...memories];
    setMemories([]);

    try {
      await clearMemories();
    } catch {
      setMemories(previous);
    }
  };

  // Handle Clear History
  const handleClearHistory = async () => {
    try {
      await newSession();
      setConfirmClearHistory(false);
      setHistoryClearedMsg(true);
      setTimeout(() => setHistoryClearedMsg(false), 3000);
    } catch {
      setConfirmClearHistory(false);
    }
  };

  const handleOpenGithub = async () => {
    try {
      await openUrl("https://github.com/openmate");
    } catch {
      window.open("https://github.com/openmate", "_blank");
    }
  };

  if (!isOpen) return null;

  const screenPerm = permissions.find((p) => p.capability === "screen_capture")?.state || "off";

  // Handle Identity Save [Feature 1.1]
  const handleSaveIdentity = async () => {
    setIsSavingIdentity(true);
    try {
      await setCompanionName(companionName.trim() || "OpenMate");
      await setUserName(userName.trim());
      setIdentitySavedMsg(true);
      setTimeout(() => setIdentitySavedMsg(false), 2500);
    } catch {
      // Identity save error handling
    } finally {
      setIsSavingIdentity(false);
    }
  };

  return (
    <motion.div
      initial={{ opacity: 0, y: "100%" }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: "100%" }}
      transition={{ duration: 0.3, ease: "easeOut" }}
      className="absolute inset-3 bg-darkBg text-white z-50 overflow-y-auto flex flex-col p-6 shadow-2xl rounded-2xl border border-surface-border"
      style={{ pointerEvents: "auto" }}
    >
      {/* Header */}
      <div className="flex items-center justify-between pb-6 border-b border-surface-border mb-6">
        <div>
          <h1 className="text-xl font-bold tracking-tight">Settings</h1>
          <p className="text-xs text-neutral-400">Configure your companion preferences and privacy controls.</p>
        </div>
        <button
          type="button"
          onClick={onClose}
          className="py-1.5 px-3 bg-surface-card hover:bg-surface-border border border-surface-border text-neutral-300 text-xs font-medium rounded-xl transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
        >
          Done
        </button>
      </div>

      {/* Developer Mode Persistent Warning Banner [DR-043] */}
      {developerMode && (
        <div className="mb-6 bg-amber-500/15 border border-amber-500/40 rounded-2xl p-4 flex items-center gap-3 text-amber-300 text-xs">
          <span className="text-lg">⚠</span>
          <div>
            <span className="font-semibold block">Developer Mode is on. Unsigned plugins are allowed.</span>
            <p className="text-amber-300/80 mt-0.5">
              Only run plugins you built yourself. Every tool execution requires confirmation.
            </p>
          </div>
        </div>
      )}

      {/* Developer Mode Confirmation Dialog Modal [DR-043] */}
      {showDevModeModal && (
        <div className="fixed inset-0 z-50 bg-black/70 flex items-center justify-center p-4">
          <div className="bg-surface-elevated border border-surface-border rounded-2xl p-6 max-w-md w-full space-y-4 shadow-2xl">
            <div className="flex items-center gap-3 text-amber-400">
              <span className="text-2xl">⚠</span>
              <h3 className="text-sm font-bold text-white uppercase tracking-wider">
                Enable Developer Mode?
              </h3>
            </div>
            <p className="text-xs text-neutral-300 leading-relaxed">
              Developer Mode allows plugins without signatures.
              Only enable this if you are building or testing your own plugins.
              Never enable this for plugins from untrusted sources.
            </p>
            <div className="flex items-center justify-end gap-3 pt-2">
              <button
                type="button"
                onClick={() => setShowDevModeModal(false)}
                className="px-3 py-1.5 rounded-xl border border-surface-border text-neutral-300 text-xs font-medium hover:bg-surface-card"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={async () => {
                  try {
                    await setDeveloperMode(true);
                    setDeveloperModeState(true);
                    setShowDevModeModal(false);
                  } catch (err) {
                    console.error("Failed to enable developer mode:", err);
                  }
                }}
                className="px-3 py-1.5 rounded-xl bg-amber-500 hover:bg-amber-600 text-black font-semibold text-xs"
              >
                Enable Developer Mode
              </button>
            </div>
          </div>
        </div>
      )}

      <div className="space-y-6 flex-1 pb-12">
        {/* COMPANION IDENTITY Section [Feature 1.1] */}
        <section className="bg-surface-elevated border border-surface-border rounded-2xl p-5 space-y-4">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-semibold uppercase tracking-wider text-neutral-400">
              Companion Identity
            </h2>
            {identitySavedMsg && (
              <span className="text-xs text-green-400 animate-pulse font-medium">
                Saved!
              </span>
            )}
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div>
              <label
                htmlFor="settings-companion-name"
                className="block text-xs font-medium text-neutral-300 mb-1.5"
              >
                Companion Name
              </label>
              <input
                id="settings-companion-name"
                type="text"
                value={companionName}
                onChange={(e) => setCompanionNameState(e.target.value)}
                placeholder="e.g. OpenMate, Hello Kitty, Luna"
                className="w-full px-3 py-2 bg-surface-card border border-surface-border rounded-xl text-xs text-white placeholder-neutral-500 focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
              />
            </div>

            <div>
              <label
                htmlFor="settings-user-name"
                className="block text-xs font-medium text-neutral-300 mb-1.5"
              >
                Your Name
              </label>
              <input
                id="settings-user-name"
                type="text"
                value={userName}
                onChange={(e) => setUserNameState(e.target.value)}
                placeholder="Your name"
                className="w-full px-3 py-2 bg-surface-card border border-surface-border rounded-xl text-xs text-white placeholder-neutral-500 focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
              />
            </div>
          </div>

          <div className="flex justify-end pt-1">
            <button
              type="button"
              onClick={handleSaveIdentity}
              disabled={isSavingIdentity}
              className="py-1.5 px-4 bg-accent hover:bg-accent-hover disabled:opacity-50 text-white text-xs font-medium rounded-xl transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
            >
              {isSavingIdentity ? "Saving..." : "Save Identity"}
            </button>
          </div>
        </section>

        {/* 4.1 API Key Section */}
        <section className="bg-surface-elevated border border-surface-border rounded-2xl p-5 space-y-4">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-semibold uppercase tracking-wider text-neutral-400">
              API Key
            </h2>
            <span
              className={`text-xs px-2 py-0.5 rounded-full font-medium ${
                apiKeySet
                  ? "bg-status-allow/20 text-status-allow"
                  : "bg-status-off/20 text-neutral-400"
              }`}
            >
              {apiKeySet ? "Configured" : "Not Set"}
            </span>
          </div>

          <div className="space-y-0.5">
            <span className="text-xs font-medium text-white block">Gemini API Key</span>
            <p className="text-xs text-neutral-400">
              {apiKeySet ? "API key configured in OS keychain" : "No API key set"}
            </p>
          </div>

          {!isReplacingKey ? (
            <div className="flex gap-2 pt-2">
              <button
                type="button"
                onClick={() => setIsReplacingKey(true)}
                className="py-1.5 px-3 bg-surface-card hover:bg-surface-border border border-surface-border text-neutral-200 text-xs font-medium rounded-xl transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
              >
                Replace Key
              </button>
              {apiKeySet && !confirmDeleteKey && (
                <button
                  type="button"
                  onClick={() => setConfirmDeleteKey(true)}
                  className="py-1.5 px-3 text-status-error hover:bg-status-error/10 border border-transparent hover:border-status-error/20 text-xs font-medium rounded-xl transition-colors focus-visible:ring-2 focus-visible:ring-status-error focus-visible:outline-none"
                >
                  Remove Key
                </button>
              )}
            </div>
          ) : (
            <div className="space-y-3 pt-2 bg-surface-card border border-surface-border rounded-xl p-3">
              <div>
                <label className="block text-xs font-medium text-neutral-300 mb-1">
                  Enter new Google Gemini API Key
                </label>
                <input
                  type="password"
                  autoComplete="off"
                  spellCheck={false}
                  placeholder="AIza..."
                  value={newKeyInput}
                  onChange={(e) => setNewKeyInput(e.target.value)}
                  className="w-full px-3 py-2 bg-darkBg border border-surface-border rounded-lg text-xs text-white placeholder-neutral-500 focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
                />
                {keyError && <p className="text-xs text-status-error mt-1">{keyError}</p>}
              </div>

              <div className="flex justify-end gap-2">
                <button
                  type="button"
                  onClick={() => {
                    setIsReplacingKey(false);
                    setNewKeyInput("");
                    setKeyError(null);
                  }}
                  className="px-3 py-1.5 text-xs text-neutral-400 hover:text-white rounded-lg focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
                >
                  Cancel
                </button>
                <button
                  type="button"
                  onClick={handleSaveNewKey}
                  disabled={isSavingKey || !newKeyInput.trim()}
                  className="px-3 py-1.5 bg-accent hover:bg-accent-hover disabled:opacity-50 text-white text-xs font-medium rounded-lg transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
                >
                  {isSavingKey ? "Saving..." : "Save Key"}
                </button>
              </div>
            </div>
          )}

          {confirmDeleteKey && (
            <div className="flex items-center gap-3 bg-status-error/10 border border-status-error/20 p-3 rounded-xl mt-2">
              <span className="text-xs text-status-error font-medium">Remove key from keychain?</span>
              <button
                type="button"
                onClick={handleDeleteKey}
                className="px-2.5 py-1 bg-status-error text-white text-xs font-medium rounded-lg hover:bg-red-600 focus-visible:ring-2 focus-visible:ring-status-error focus-visible:outline-none"
              >
                Yes, Remove
              </button>
              <button
                type="button"
                onClick={() => setConfirmDeleteKey(false)}
                className="px-2.5 py-1 text-xs text-neutral-300 hover:text-white rounded-lg hover:bg-surface-card focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
              >
                Cancel
              </button>
            </div>
          )}
        </section>

        {/* 4.2 Permissions Section */}
        <section className="bg-surface-elevated border border-surface-border rounded-2xl p-5 space-y-4">
          <h2 className="text-sm font-semibold uppercase tracking-wider text-neutral-400">
            Permissions
          </h2>

          <div className="space-y-4">
            {/* Screen Awareness */}
            <div className="flex items-start justify-between gap-4">
              <div className="space-y-0.5">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-medium text-white">Screen Awareness</span>
                  <span
                    className={`text-[10px] px-1.5 py-0.2 rounded font-medium ${
                      screenPerm === "allow"
                        ? "bg-status-allow/20 text-status-allow"
                        : "bg-neutral-800 text-neutral-400"
                    }`}
                  >
                    {screenPerm.toUpperCase()}
                  </span>
                </div>
                <p className="text-xs text-neutral-400">
                  Lets OpenMate see active window context to provide relevant answers. Screenshots are never saved to disk.
                </p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={screenPerm === "allow"}
                onClick={() => handleTogglePermission("screen_capture", screenPerm)}
                className={`relative inline-flex h-5 w-9 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none ${
                  screenPerm === "allow" ? "bg-accent" : "bg-neutral-700"
                }`}
              >
                <span
                  className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow transition duration-200 ease-in-out ${
                    screenPerm === "allow" ? "translate-x-4" : "translate-x-0"
                  }`}
                />
              </button>
            </div>

            {/* Microphone */}
            <div className="border-t border-surface-border pt-3 flex items-start justify-between gap-4">
              <div className="space-y-0.5">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-medium text-white">Microphone</span>
                  <span
                    className={`text-[10px] px-1.5 py-0.2 rounded font-medium ${
                      (permissions.find((p) => p.capability === "microphone")?.state || "off") === "allow"
                        ? "bg-status-allow/20 text-status-allow"
                        : "bg-neutral-800 text-neutral-400"
                    }`}
                  >
                    {(permissions.find((p) => p.capability === "microphone")?.state || "off").toUpperCase()}
                  </span>
                </div>
                <p className="text-xs text-neutral-400">
                  Lets you speak to OpenMate for voice conversation and automatic speech transcription. Audio is never saved to disk.
                </p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={(permissions.find((p) => p.capability === "microphone")?.state || "off") === "allow"}
                onClick={() =>
                  handleTogglePermission(
                    "microphone",
                    permissions.find((p) => p.capability === "microphone")?.state || "off"
                  )
                }
                className={`relative inline-flex h-5 w-9 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none ${
                  (permissions.find((p) => p.capability === "microphone")?.state || "off") === "allow"
                    ? "bg-accent"
                    : "bg-neutral-700"
                }`}
              >
                <span
                  className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow transition duration-200 ease-in-out ${
                    (permissions.find((p) => p.capability === "microphone")?.state || "off") === "allow"
                      ? "translate-x-4"
                      : "translate-x-0"
                  }`}
                />
              </button>
            </div>

            {/* Clipboard Access */}
            <div className="border-t border-surface-border pt-3 flex items-start justify-between gap-4">
              <div className="space-y-0.5">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-medium text-white">Clipboard Access</span>
                  <span
                    className={`text-[10px] px-1.5 py-0.2 rounded font-medium ${
                      (permissions.find((p) => p.capability === "clipboard")?.state || "off") === "allow"
                        ? "bg-status-allow/20 text-status-allow"
                        : "bg-neutral-800 text-neutral-400"
                    }`}
                  >
                    {(permissions.find((p) => p.capability === "clipboard")?.state || "off").toUpperCase()}
                  </span>
                </div>
                <p className="text-xs text-neutral-400">
                  Lets OpenMate know when you copy something. Content is never sent to Gemini unless you ask.
                </p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={(permissions.find((p) => p.capability === "clipboard")?.state || "off") === "allow"}
                onClick={() =>
                  handleTogglePermission(
                    "clipboard",
                    permissions.find((p) => p.capability === "clipboard")?.state || "off"
                  )
                }
                className={`relative inline-flex h-5 w-9 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none ${
                  (permissions.find((p) => p.capability === "clipboard")?.state || "off") === "allow"
                    ? "bg-accent"
                    : "bg-neutral-700"
                }`}
              >
                <span
                  className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow transition duration-200 ease-in-out ${
                    (permissions.find((p) => p.capability === "clipboard")?.state || "off") === "allow"
                      ? "translate-x-4"
                      : "translate-x-0"
                  }`}
                />
              </button>
            </div>

            {/* Application Launch [Feature 2-A / Fix] */}
            <div className="border-t border-surface-border pt-3 flex items-start justify-between gap-4">
              <div className="space-y-0.5">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-medium text-white">Application Launch</span>
                  <span
                    className={`text-[10px] px-1.5 py-0.2 rounded font-medium ${
                      (permissions.find((p) => p.capability === "app_launch")?.state || "off") === "allow"
                        ? "bg-status-allow/20 text-status-allow"
                        : "bg-neutral-800 text-neutral-400"
                    }`}
                  >
                    {(permissions.find((p) => p.capability === "app_launch")?.state || "off").toUpperCase()}
                  </span>
                </div>
                <p className="text-xs text-neutral-400">
                  Allows OpenMate to open desktop applications on your command (e.g. "open Safari", "open VS Code").
                </p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={(permissions.find((p) => p.capability === "app_launch")?.state || "off") === "allow"}
                onClick={() =>
                  handleTogglePermission(
                    "app_launch",
                    permissions.find((p) => p.capability === "app_launch")?.state || "off"
                  )
                }
                className={`relative inline-flex h-5 w-9 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none ${
                  (permissions.find((p) => p.capability === "app_launch")?.state || "off") === "allow"
                    ? "bg-accent"
                    : "bg-neutral-700"
                }`}
              >
                <span
                  className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow transition duration-200 ease-in-out ${
                    (permissions.find((p) => p.capability === "app_launch")?.state || "off") === "allow"
                      ? "translate-x-4"
                      : "translate-x-0"
                  }`}
                />
              </button>
            </div>

            {/* Filesystem Read */}
            <div className="border-t border-surface-border pt-3 flex items-start justify-between gap-4">
              <div className="space-y-0.5">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-medium text-white">Filesystem Read</span>
                  <span
                    className={`text-[10px] px-1.5 py-0.2 rounded font-medium ${
                      (permissions.find((p) => p.capability === "filesystem_read")?.state || "off") === "allow"
                        ? "bg-status-allow/20 text-status-allow"
                        : "bg-neutral-800 text-neutral-400"
                    }`}
                  >
                    {(permissions.find((p) => p.capability === "filesystem_read")?.state || "off").toUpperCase()}
                  </span>
                </div>
                <p className="text-xs text-neutral-400">
                  Allows OpenMate to read code files and directory listings when you ask questions about your project files.
                </p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={(permissions.find((p) => p.capability === "filesystem_read")?.state || "off") === "allow"}
                onClick={() =>
                  handleTogglePermission(
                    "filesystem_read",
                    permissions.find((p) => p.capability === "filesystem_read")?.state || "off"
                  )
                }
                className={`relative inline-flex h-5 w-9 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none ${
                  (permissions.find((p) => p.capability === "filesystem_read")?.state || "off") === "allow"
                    ? "bg-accent"
                    : "bg-neutral-700"
                }`}
              >
                <span
                  className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow transition duration-200 ease-in-out ${
                    (permissions.find((p) => p.capability === "filesystem_read")?.state || "off") === "allow"
                      ? "translate-x-4"
                      : "translate-x-0"
                  }`}
                />
              </button>
            </div>

            {/* Filesystem Write */}
            <div className="border-t border-surface-border pt-3 flex items-start justify-between gap-4">
              <div className="space-y-0.5">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-medium text-white">Filesystem Write</span>
                  <span
                    className={`text-[10px] px-1.5 py-0.2 rounded font-medium ${
                      (permissions.find((p) => p.capability === "filesystem_write")?.state || "off") === "allow"
                        ? "bg-status-allow/20 text-status-allow"
                        : "bg-neutral-800 text-neutral-400"
                    }`}
                  >
                    {(permissions.find((p) => p.capability === "filesystem_write")?.state || "off").toUpperCase()}
                  </span>
                </div>
                <p className="text-xs text-neutral-400">
                  Allows OpenMate to create new files on your request. Never overwrites existing files without confirmation.
                </p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={(permissions.find((p) => p.capability === "filesystem_write")?.state || "off") === "allow"}
                onClick={() =>
                  handleTogglePermission(
                    "filesystem_write",
                    permissions.find((p) => p.capability === "filesystem_write")?.state || "off"
                  )
                }
                className={`relative inline-flex h-5 w-9 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none ${
                  (permissions.find((p) => p.capability === "filesystem_write")?.state || "off") === "allow"
                    ? "bg-accent"
                    : "bg-neutral-700"
                }`}
              >
                <span
                  className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow transition duration-200 ease-in-out ${
                    (permissions.find((p) => p.capability === "filesystem_write")?.state || "off") === "allow"
                      ? "translate-x-4"
                      : "translate-x-0"
                  }`}
                />
              </button>
            </div>
          </div>
        </section>

        {/* 4.3 Proactive Assistance Section [DR-017] */}
        <section className="bg-surface-elevated border border-surface-border rounded-2xl p-5 space-y-4">
          <h2 className="text-sm font-semibold uppercase tracking-wider text-neutral-400">
            Proactive Assistance
          </h2>
          
          <div className="grid grid-cols-3 gap-2 bg-darkBg p-1.5 rounded-xl border border-surface-border">
            {(["off", "subtle", "active"] as const).map((mode) => (
              <button
                key={mode}
                type="button"
                onClick={async () => {
                  setProactiveModeState(mode);
                  try {
                    await setProactiveMode(mode);
                  } catch {
                    // Non-blocking
                  }
                }}
                className={`py-2 px-3 text-xs font-medium rounded-lg capitalize transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none ${
                  proactiveMode === mode
                    ? "bg-accent text-white shadow-sm"
                    : "text-neutral-400 hover:text-neutral-200 hover:bg-surface-elevated"
                }`}
              >
                {mode}
              </button>
            ))}
          </div>

          <div className="text-xs text-neutral-400 bg-surface-card p-3 rounded-xl border border-surface-border">
            {proactiveMode === "off" && (
              <p>Companion only responds when you start the conversation.</p>
            )}
            {proactiveMode === "subtle" && (
              <p>Companion offers help when you return after a break or switch context.</p>
            )}
            {proactiveMode === "active" && (
              <p>Companion proactively notices context changes and offers help.</p>
            )}
          </div>
        </section>

        {/* 4.4 Avatar Section [Phase 3-A, DR-036] */}
        <section className="bg-surface-elevated border border-surface-border rounded-2xl p-5 space-y-4">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-semibold uppercase tracking-wider text-neutral-400">
              Avatar
            </h2>
            <span className="text-xs text-neutral-400 font-medium">
              {avatars.length} available
            </span>
          </div>

          <div className="space-y-3">
            {avatars.map((avatar) => {
              const isDefault = avatar.name === "default";
              const displayName = isDefault ? "Built-in Cat" : avatar.name;
              const isActivating = activatingAvatar === avatar.name;

              return (
                <div
                  key={avatar.name}
                  className={`flex items-center justify-between gap-3 p-3.5 rounded-xl border transition-colors ${
                    avatar.is_active
                      ? "bg-accent/10 border-accent/60"
                      : "bg-surface-card border-surface-border hover:border-neutral-700"
                  }`}
                >
                  <div className="flex items-center gap-3.5 min-w-0">
                    {/* Avatar thumbnail icon */}
                    <div
                      className={`w-10 h-10 rounded-full flex-shrink-0 flex items-center justify-center border ${
                        avatar.is_active
                          ? "bg-accent/20 border-accent text-accent-light"
                          : "bg-darkBg border-surface-border text-neutral-400"
                      }`}
                    >
                      {isDefault ? (
                        <svg viewBox="0 0 36 36" className="w-6 h-6 fill-none">
                          <circle cx="18" cy="18" r="14" className="stroke-current" strokeWidth="2" />
                          <circle cx="13" cy="16" r="2" className="fill-white" />
                          <circle cx="23" cy="16" r="2" className="fill-white" />
                          <path
                            d="M14 22 Q18 25 22 22"
                            className="stroke-white"
                            strokeWidth="1.5"
                            strokeLinecap="round"
                          />
                        </svg>
                      ) : (
                        <svg
                          viewBox="0 0 24 24"
                          className="w-5 h-5 fill-none stroke-current"
                          strokeWidth="2"
                          strokeLinecap="round"
                          strokeLinejoin="round"
                        >
                          <circle cx="12" cy="8" r="5" />
                          <path d="M20 21a8 8 0 0 0-16 0" />
                        </svg>
                      )}
                    </div>

                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-xs font-semibold text-white truncate">
                          {displayName}
                        </span>
                        {avatar.author && (
                          <span className="text-[10px] text-neutral-400 flex-shrink-0">
                            by {avatar.author}
                          </span>
                        )}
                        {avatar.is_active && (
                          <span className="text-[10px] px-1.5 py-0.2 rounded-full font-medium bg-accent text-white">
                            Active
                          </span>
                        )}
                      </div>
                      <p className="text-xs text-neutral-400 truncate mt-0.5">
                        {avatar.description || "Custom avatar package"}
                      </p>
                    </div>
                  </div>

                  <div>
                    {avatar.is_active ? (
                      <span className="text-xs font-medium text-accent-light px-3 py-1.5">
                        Active
                      </span>
                    ) : (
                      <button
                        type="button"
                        disabled={isActivating}
                        onClick={async () => {
                          setActivatingAvatar(avatar.name);
                          try {
                            await setActiveAvatar(avatar.name);
                            setAvatars((prev) =>
                              prev.map((a) => ({
                                ...a,
                                is_active: a.name === avatar.name,
                              }))
                            );
                            window.dispatchEvent(new CustomEvent("openmate-avatar-changed"));
                          } catch (err) {
                            console.error("Failed to activate avatar:", err);
                          } finally {
                            setActivatingAvatar(null);
                          }
                        }}
                        className="py-1.5 px-3 bg-surface-card hover:bg-surface-border border border-surface-border text-neutral-200 text-xs font-medium rounded-xl transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none whitespace-nowrap"
                      >
                        {isActivating ? "Activating..." : "Use this avatar"}
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>

          <div className="text-xs text-neutral-400 bg-surface-card/60 p-3 rounded-xl border border-surface-border/80">
            <p>
              To add community avatars, place them in the <code className="text-accent-light font-mono">avatars/</code> folder in the OpenMate directory.
            </p>
          </div>
        </section>

        {/* 4.4 Native Tool Plugins Section [Phase 3-D, DR-039 through DR-044] */}
        <section className="bg-surface-elevated border border-surface-border rounded-2xl p-5 space-y-4">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-semibold uppercase tracking-wider text-neutral-400">
              Native Tool Plugins
            </h2>
            <span className="text-xs text-neutral-400 font-medium">
              {plugins.length} installed
            </span>
          </div>

          {/* Developer Mode Toggle */}
          <div className="flex items-center justify-between p-3.5 bg-surface-card rounded-xl border border-surface-border">
            <div>
              <span className="text-xs font-semibold text-white block">
                Developer Mode (Unsigned Plugins)
              </span>
              <p className="text-xs text-neutral-400 mt-0.5">
                Allow loading local experimental plugins without signatures.
              </p>
            </div>
            <button
              type="button"
              onClick={async () => {
                if (!developerMode) {
                  setShowDevModeModal(true);
                } else {
                  try {
                    await setDeveloperMode(false);
                    setDeveloperModeState(false);
                  } catch (err) {
                    console.error("Failed to disable developer mode:", err);
                  }
                }
              }}
              className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                developerMode ? "bg-amber-500" : "bg-darkBg border border-surface-border"
              }`}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                  developerMode ? "translate-x-6" : "translate-x-1"
                }`}
              />
            </button>
          </div>

          {/* Plugin List */}
          <div className="space-y-3">
            {plugins.length === 0 ? (
              <div className="text-center py-6 text-neutral-500 text-xs bg-surface-card/40 rounded-xl border border-surface-border/60">
                No native plugins installed yet.
              </div>
            ) : (
              plugins.map((plugin) => {
                const badgeClass =
                  plugin.trust_level === "builtin"
                    ? "bg-emerald-500/20 text-emerald-300 border-emerald-500/40"
                    : plugin.trust_level === "community"
                    ? "bg-sky-500/20 text-sky-300 border-sky-500/40"
                    : plugin.trust_level === "user_approved"
                    ? "bg-amber-500/20 text-amber-300 border-amber-500/40"
                    : "bg-rose-500/20 text-rose-300 border-rose-500/40";

                const badgeLabel =
                  plugin.trust_level === "builtin"
                    ? "OpenMate"
                    : plugin.trust_level === "community"
                    ? "Community Verified"
                    : plugin.trust_level === "user_approved"
                    ? "User Approved"
                    : "Unsigned";

                const trustExplanation =
                  plugin.trust_level === "builtin"
                    ? "Built-in core native tool"
                    : plugin.trust_level === "community"
                    ? "Signature verified against OpenMate's trusted registry"
                    : plugin.trust_level === "user_approved"
                    ? "You manually approved this author's signature"
                    : "This plugin has no verified signature. Only enable if you built it.";

                const isEnabled = enabledPlugins[plugin.id] ?? true;

                return (
                  <div
                    key={plugin.id}
                    className="p-4 rounded-xl border border-surface-border bg-surface-card space-y-3"
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        <div className="flex items-center gap-2 flex-wrap">
                          <span className="text-xs font-semibold text-white">
                            {plugin.name}
                          </span>
                          <span className="text-[10px] text-neutral-400 font-mono">
                            v{plugin.version}
                          </span>
                          <span className="text-[10px] text-neutral-400">
                            by {plugin.author}
                          </span>
                          <span
                            className={`text-[10px] px-2 py-0.5 rounded-full font-medium border ${badgeClass}`}
                          >
                            {badgeLabel}
                          </span>
                        </div>
                        <p className="text-xs text-neutral-300 mt-1">
                          {plugin.description}
                        </p>
                        <p className="text-[11px] text-neutral-500 mt-1 italic">
                          {trustExplanation}
                        </p>
                      </div>

                      <div className="flex items-center gap-2 flex-shrink-0">
                        {developerMode && plugin.trust_level === "unknown" && plugin.author_pubkey && (
                          <button
                            type="button"
                            onClick={async () => {
                              try {
                                await approvePluginKey(plugin.author_pubkey);
                                const updated = await listPlugins();
                                setPlugins(updated);
                              } catch (err) {
                                console.error("Failed to approve key:", err);
                              }
                            }}
                            className="text-xs px-2 py-1 rounded-lg border border-amber-500/40 bg-amber-500/10 text-amber-300 hover:bg-amber-500/20 font-medium transition-colors"
                            title="Approve Author Public Key"
                          >
                            Trust Author
                          </button>
                        )}

                        <button
                          type="button"
                          onClick={() =>
                            setEnabledPlugins((prev) => ({
                              ...prev,
                              [plugin.id]: !isEnabled,
                            }))
                          }
                          className={`text-xs px-2.5 py-1 rounded-lg border font-medium transition-colors ${
                            isEnabled
                              ? "bg-accent/20 border-accent text-accent-light"
                              : "bg-darkBg border-surface-border text-neutral-400"
                          }`}
                        >
                          {isEnabled ? "Enabled" : "Disabled"}
                        </button>

                        <button
                          type="button"
                          onClick={async () => {
                            try {
                              await removePlugin(plugin.id);
                              setPlugins((prev) =>
                                prev.filter((p) => p.id !== plugin.id)
                              );
                            } catch (err) {
                              console.error("Failed to remove plugin:", err);
                            }
                          }}
                          className="text-xs p-1 text-neutral-500 hover:text-rose-400 transition-colors"
                          title="Remove Plugin"
                        >
                          ✕
                        </button>
                      </div>
                    </div>

                    {plugin.tools.length > 0 && (
                      <div className="pt-2 border-t border-surface-border/60 flex items-center gap-1.5 flex-wrap">
                        <span className="text-[10px] uppercase font-semibold text-neutral-500">
                          Tools:
                        </span>
                        {plugin.tools.map((tool) => (
                          <span
                            key={tool.name}
                            className="text-[10px] bg-darkBg border border-surface-border px-2 py-0.5 rounded font-mono text-neutral-300"
                            title={tool.description}
                          >
                            {tool.name}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>
                );
              })
            )}
          </div>

          <div className="text-xs text-neutral-400 bg-surface-card/60 p-3 rounded-xl border border-surface-border/80">
            <p>
              To install a plugin, place its folder in the <code className="text-accent-light font-mono">plugins/</code> folder in the OpenMate directory. Then restart OpenMate.
            </p>
          </div>
        </section>

        {/* 4.5 Memory Section */}
        <section className="bg-surface-elevated border border-surface-border rounded-2xl p-5 space-y-4">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-semibold uppercase tracking-wider text-neutral-400">
              Memory & History
            </h2>
            <button
              type="button"
              onClick={() => setIsMemoryExpanded((prev) => !prev)}
              className="text-xs text-accent-light hover:underline focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none rounded"
            >
              {isMemoryExpanded ? "Hide Memories" : `View Memories (${memories.length})`}
            </button>
          </div>

          {isMemoryExpanded && (
            <div className="bg-surface-card border border-surface-border rounded-xl p-3">
              <MemoryPanel
                memories={memories}
                onDelete={handleDeleteMemory}
                onClearAll={handleClearAllMemories}
              />
            </div>
          )}

          <div className="pt-2 border-t border-surface-border flex items-center justify-between">
            <div>
              <span className="text-xs font-medium text-white block">Conversation Session</span>
              <p className="text-xs text-neutral-400">Reset active conversation context.</p>
            </div>

            {!confirmClearHistory ? (
              <button
                type="button"
                onClick={() => setConfirmClearHistory(true)}
                className="py-1 px-2.5 bg-surface-card hover:bg-surface-border border border-surface-border text-neutral-300 text-xs font-medium rounded-lg transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
              >
                New Session
              </button>
            ) : (
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={handleClearHistory}
                  className="px-2 py-1 bg-accent text-white text-xs font-medium rounded-lg hover:bg-accent-hover focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
                >
                  Confirm Reset
                </button>
                <button
                  type="button"
                  onClick={() => setConfirmClearHistory(false)}
                  className="px-2 py-1 text-xs text-neutral-400 hover:text-white rounded-lg focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
                >
                  Cancel
                </button>
              </div>
            )}
          </div>

          {historyClearedMsg && (
            <p className="text-xs text-status-allow">New conversation session started.</p>
          )}
        </section>

        {/* 4.5 About Section */}
        <section className="bg-surface-elevated border border-surface-border rounded-2xl p-5 space-y-2">
          <h2 className="text-sm font-semibold uppercase tracking-wider text-neutral-400">
            About OpenMate
          </h2>
          <div className="flex items-center justify-between text-xs text-neutral-300">
            <span>Version</span>
            <span className="font-mono text-neutral-400">0.1.0</span>
          </div>
          <div className="flex items-center justify-between text-xs text-neutral-300">
            <span>License</span>
            <span className="text-neutral-400">MIT License</span>
          </div>
          <div className="pt-2 border-t border-surface-border flex justify-end">
            <button
              type="button"
              onClick={handleOpenGithub}
              className="text-xs text-accent-light hover:underline focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none rounded"
            >
              GitHub Repository
            </button>
          </div>
        </section>
      </div>
    </motion.div>
  );
}
