/**
 * OpenMate Onboarding Flow
 *
 * Three-screen setup flow:
 * 1. Welcome & capability summary
 * 2. API key configuration (stored exclusively in OS keychain) [DR-027, DR-011]
 * 3. Initial permission preferences [DR-012]
 */

import { useState, useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  setApiKey,
  getPermissions,
  setPermission,
  getUserName,
  setUserName,
  getCompanionName,
  setCompanionName,
} from "../../hooks/useIpc";
import type { PermissionState } from "../../types";

interface OnboardingProps {
  onComplete: () => void;
}

export default function Onboarding({ onComplete }: OnboardingProps) {
  const [step, setStep] = useState<1 | 2 | 3>(1);

  // Screen 1 state: Name customisation [Feature 1.3]
  const [userName, setUserNameInput] = useState("");
  const [companionName, setCompanionNameInput] = useState("OpenMate");

  // Screen 2 state: key is cleared immediately after the IPC call
  const [keyValue, setKeyValue] = useState("");
  const [isSavingKey, setIsSavingKey] = useState(false);
  const [keyError, setKeyError] = useState<string | null>(null);

  // Screen 3 state: permissions
  const [screenCaptureAllowed, setScreenCaptureAllowed] = useState(false);
  const [isSavingPermissions, setIsSavingPermissions] = useState(false);

  useEffect(() => {
    getUserName().then((name) => {
      if (name) setUserNameInput(name);
    }).catch(() => {});
    getCompanionName().then((name) => {
      if (name) setCompanionNameInput(name);
    }).catch(() => {});
  }, []);

  useEffect(() => {
    if (step === 3) {
      getPermissions()
        .then((perms) => {
          const screenPerm = perms.find((p) => p.capability === "screen_capture");
          if (screenPerm) {
            setScreenCaptureAllowed(screenPerm.state === "allow");
          }
        })
        .catch(() => {});
    }
  }, [step]);

  const handleOpenAiStudio = async () => {
    try {
      await openUrl("https://aistudio.google.com/app/apikey");
    } catch {
      window.open("https://aistudio.google.com/app/apikey", "_blank");
    }
  };

  const handleStep1Next = async () => {
    if (userName.trim()) {
      await setUserName(userName.trim()).catch(() => {});
    }
    await setCompanionName(companionName.trim() || "OpenMate").catch(() => {});
    setStep(2);
  };

  const handleSaveKey = async () => {
    const trimmed = keyValue.trim();
    if (!trimmed) {
      setKeyError("API key cannot be empty.");
      setKeyValue("");
      return;
    }

    setIsSavingKey(true);
    setKeyError(null);

    try {
      await setApiKey(trimmed);
      // SECURITY: Clear key state immediately after IPC call completes
      setKeyValue("");
      setStep(3);
    } catch (err) {
      // Clear key input on error as well for security
      setKeyValue("");
      setKeyError("Failed to save key: " + String(err));
    } finally {
      setIsSavingKey(false);
    }
  };

  const handleFinishOnboarding = async () => {
    setIsSavingPermissions(true);
    try {
      const screenState: PermissionState = screenCaptureAllowed ? "allow" : "off";
      await setPermission("screen_capture", screenState);
      onComplete();
    } catch {
      onComplete();
    } finally {
      setIsSavingPermissions(false);
    }
  };

  const slideVariants = {
    initial: { opacity: 0, x: 20 },
    animate: { opacity: 1, x: 0 },
    exit: { opacity: 0, x: -20 },
  };

  return (
    <div className="flex items-center justify-center min-h-screen bg-darkBg text-white p-6">
      <div className="w-full max-w-md bg-surface-elevated border border-surface-border rounded-2xl p-8 shadow-2xl">
        <AnimatePresence mode="wait">
          {step === 1 && (
            <motion.div
              key="step-1"
              variants={slideVariants}
              initial="initial"
              animate="animate"
              exit="exit"
              transition={{ duration: 0.25 }}
              className="space-y-5"
            >
              <div>
                <h1 className="text-2xl font-bold tracking-tight text-white mb-1.5">
                  Meet Your Companion
                </h1>
                <p className="text-xs text-neutral-400 leading-relaxed">
                  Your open-source AI desktop companion for coding, productivity, and conversation.
                </p>
              </div>

              {/* Name customisation inputs [Feature 1.3] */}
              <div className="space-y-3 bg-surface-card border border-surface-border rounded-xl p-4">
                <div>
                  <label
                    htmlFor="user-name-input"
                    className="block text-xs font-medium text-neutral-300 mb-1"
                  >
                    What's your name?
                  </label>
                  <input
                    id="user-name-input"
                    type="text"
                    placeholder="Your name"
                    value={userName}
                    onChange={(e) => setUserNameInput(e.target.value)}
                    className="w-full px-3 py-2 bg-surface-elevated border border-surface-border rounded-lg text-sm text-white placeholder-neutral-500 focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
                  />
                </div>

                <div>
                  <label
                    htmlFor="companion-name-input"
                    className="block text-xs font-medium text-neutral-300 mb-1"
                  >
                    What will you name your companion?
                  </label>
                  <input
                    id="companion-name-input"
                    type="text"
                    placeholder="e.g. OpenMate, Hello Kitty, Luna"
                    value={companionName}
                    onChange={(e) => setCompanionNameInput(e.target.value)}
                    className="w-full px-3 py-2 bg-surface-elevated border border-surface-border rounded-lg text-sm text-white placeholder-neutral-500 focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
                  />
                </div>
              </div>

              <div className="space-y-2 bg-surface-card/60 border border-surface-border rounded-xl p-3 text-xs text-neutral-300">
                <div className="flex items-start gap-2">
                  <span className="text-accent">•</span>
                  <span>Assists with coding, research, and desktop tasks</span>
                </div>
                <div className="flex items-start gap-2">
                  <span className="text-accent">•</span>
                  <span>100% private: uses your Gemini key & local database</span>
                </div>
              </div>

              <button
                type="button"
                onClick={handleStep1Next}
                className="w-full py-2.5 px-4 bg-accent hover:bg-accent-hover text-white text-sm font-medium rounded-xl focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none transition-colors"
              >
                Get Started
              </button>
            </motion.div>
          )}

          {step === 2 && (
            <motion.div
              key="step-2"
              variants={slideVariants}
              initial="initial"
              animate="animate"
              exit="exit"
              transition={{ duration: 0.25 }}
              className="space-y-6"
            >
              <div>
                <h2 className="text-xl font-bold tracking-tight text-white mb-2">
                  Connect your Gemini API key
                </h2>
                <p className="text-xs text-neutral-400 leading-relaxed">
                  Your key is stored securely on this device using your OS keychain. It is never sent to OpenMate servers.
                </p>
              </div>

              <div className="space-y-3">
                <div>
                  <label
                    htmlFor="api-key-input"
                    className="block text-xs font-medium text-neutral-300 mb-1.5"
                  >
                    Google Gemini API Key
                  </label>
                  <input
                    id="api-key-input"
                    type="password"
                    autoComplete="off"
                    spellCheck={false}
                    placeholder="AIza..."
                    value={keyValue}
                    onChange={(e) => setKeyValue(e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && handleSaveKey()}
                    className="w-full px-3.5 py-2.5 bg-surface-card border border-surface-border rounded-xl text-sm text-white placeholder-neutral-500 focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
                  />
                  {keyError && (
                    <p className="text-xs text-status-error mt-2">{keyError}</p>
                  )}
                </div>

                <div className="flex justify-end">
                  <button
                    type="button"
                    onClick={handleOpenAiStudio}
                    className="text-xs text-accent-light hover:underline focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none rounded"
                  >
                    Where do I get a key?
                  </button>
                </div>
              </div>

              <div className="flex gap-3">
                <button
                  type="button"
                  onClick={() => {
                    setKeyValue("");
                    setKeyError(null);
                    setStep(1);
                  }}
                  className="w-1/3 py-2.5 px-4 bg-surface-card hover:bg-surface-border border border-surface-border text-neutral-300 text-sm font-medium rounded-xl focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none transition-colors"
                >
                  Back
                </button>
                <button
                  type="button"
                  onClick={handleSaveKey}
                  disabled={isSavingKey || !keyValue.trim()}
                  className="w-2/3 py-2.5 px-4 bg-accent hover:bg-accent-hover disabled:opacity-50 disabled:cursor-not-allowed text-white text-sm font-medium rounded-xl focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none transition-colors"
                >
                  {isSavingKey ? "Saving..." : "Save Key"}
                </button>
              </div>
            </motion.div>
          )}

          {step === 3 && (
            <motion.div
              key="step-3"
              variants={slideVariants}
              initial="initial"
              animate="animate"
              exit="exit"
              transition={{ duration: 0.25 }}
              className="space-y-6"
            >
              <div>
                <h2 className="text-xl font-bold tracking-tight text-white mb-2">
                  Choose what OpenMate can access
                </h2>
                <p className="text-xs text-neutral-400 leading-relaxed">
                  You can change these permissions anytime in Settings.
                </p>
              </div>

              <div className="space-y-4 bg-surface-card border border-surface-border rounded-xl p-4">
                {/* Screen Awareness Toggle */}
                <div className="flex items-start justify-between gap-4">
                  <div className="space-y-1">
                    <span className="text-sm font-medium text-white block">
                      Screen Awareness
                    </span>
                    <p className="text-xs text-neutral-400 leading-relaxed">
                      Lets OpenMate see what's on your screen to offer help. Screenshots are sent to Gemini and discarded immediately. Never saved.
                    </p>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={screenCaptureAllowed}
                    onClick={() => setScreenCaptureAllowed((prev) => !prev)}
                    className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none ${
                      screenCaptureAllowed ? "bg-accent" : "bg-neutral-700"
                    }`}
                  >
                    <span
                      className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
                        screenCaptureAllowed ? "translate-x-5" : "translate-x-0"
                      }`}
                    />
                  </button>
                </div>

                <div className="border-t border-surface-border pt-4">
                  {/* Microphone Toggle (Disabled - DR-005 TBD) */}
                  <div className="flex items-start justify-between gap-4 opacity-50 cursor-not-allowed">
                    <div className="space-y-1">
                      <span className="text-sm font-medium text-neutral-400 block">
                        Microphone
                      </span>
                      <p className="text-xs text-neutral-500 leading-relaxed">
                        For voice conversation. Not available yet — coming soon.
                      </p>
                    </div>
                    <div className="relative inline-flex h-6 w-11 flex-shrink-0 rounded-full border-2 border-transparent bg-neutral-800">
                      <span className="inline-block h-5 w-5 transform rounded-full bg-neutral-600 translate-x-0" />
                    </div>
                  </div>
                </div>
              </div>

              <button
                type="button"
                onClick={handleFinishOnboarding}
                disabled={isSavingPermissions}
                className="w-full py-2.5 px-4 bg-accent hover:bg-accent-hover disabled:opacity-50 text-white text-sm font-medium rounded-xl focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none transition-colors"
              >
                {isSavingPermissions ? "Starting..." : "Start"}
              </button>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </div>
  );
}
