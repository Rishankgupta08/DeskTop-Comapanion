/**
 * OpenMate Root Application
 *
 * Transparent desktop overlay shell. The Tauri window covers the full screen
 * but is transparent everywhere except the avatar, chat panel, and message bubbles.
 * pointerEvents: none on the root makes transparent areas click-through,
 * pointerEvents: all restores interactivity on the visible widgets.
 * [DR-003, DR-027, DR-011, ADR-003, Feature 1, Feature 2, Feature 3]
 */

import "./App.css";
import { useEffect, useState, useRef, useCallback } from "react";
import { AnimatePresence } from "framer-motion";
import { listen } from "@tauri-apps/api/event";
import {
  hasApiKey,
  getMode,
  getPermissions,
  getProactiveMode,
  getCompanionName,
  getUserName,
  generateAmbientMessage,
  startVoiceInput,
} from "./hooks/useIpc";
import type { AvatarState, CompanionMode, ChatMessage, ProactiveMode } from "./types";

import Onboarding from "./components/onboarding/Onboarding";
import Avatar from "./components/avatar/Avatar";
import MessageBubble from "./components/avatar/MessageBubble";
import ChatPanel, { detectSentiment } from "./components/chat/ChatPanel";
import Settings from "./components/settings/Settings";
import { initializeWindowLayout, setWindowLayoutState } from "./utils/windowLayout";

// Ambient messages pools by time of day [Feature 2.2, Step 2]
const AMBIENT_MESSAGES = {
  morning: [
    "*stretches and yawns* Good morning! Ready?",
    "Psst. I made you an imaginary coffee.",
    "Morning! What are we doing today?",
  ],
  afternoon: [
    "How's it going? Need anything?",
    "You've been working hard. I see you. 👀",
    "*bats at your cursor* Pay attention to me!",
  ],
  evening: [
    "Still here! Don't forget to rest.",
    "What did you get done today?",
    "*paws at screen* Talk to me?",
  ],
};

export default function App() {
  // App initialization & API Key gate — checked once on mount only
  const [apiKeyConfigured, setApiKeyConfigured] = useState<boolean | null>(null);

  // Companion UI state
  const [avatarState, setAvatarState] = useState<AvatarState>("idle");
  const [isChatOpen, setIsChatOpen] = useState(false);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [currentMode, setCurrentMode] = useState<CompanionMode>("assistant");
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [isMicAllowed, setIsMicAllowed] = useState(false);

  // Ambient Message Bubble state [Feature 2.1, Step 4]
  const [bubbleMessage, setBubbleMessage] = useState<string | null>(null);
  const [bubbleSender, setBubbleSender] = useState<string>("OpenMate");
  const [companionName, setCompanionName] = useState<string>("OpenMate");
  const [userName, setUserName] = useState<string>("");
  const [proactiveMode, setProactiveMode] = useState<ProactiveMode>("subtle");
  const [bubbleCardHeight, setBubbleCardHeight] = useState<number>(70);

  // Track chat activity for idle detection [Feature 2.2]
  const lastChatTimeRef = useRef<number>(Date.now());
  const ambientTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Check permissions & configuration [Step 4]
  const refreshStatus = useCallback(async () => {
    try {
      const perms = await getPermissions();
      const mic = perms.find((p) => p.capability === "microphone");
      setIsMicAllowed(mic?.state === "allow");

      const compName = await getCompanionName().catch(() => "OpenMate");
      if (compName) setCompanionName(compName);

      const uName = await getUserName().catch(() => "");
      if (uName) setUserName(uName);

      const pMode = await getProactiveMode().catch(() => "subtle" as ProactiveMode);
      if (pMode) setProactiveMode(pMode as ProactiveMode);
    } catch {
      // Non-blocking status check error handling
    }
  }, []);

  // Initial check on mount only
  useEffect(() => {
    let isMounted = true;

    async function initialCheck() {
      try {
        await initializeWindowLayout();
        const exists = await hasApiKey();
        if (isMounted) {
          setApiKeyConfigured(exists);
          if (exists) {
            const mode = await getMode();
            setCurrentMode(mode);
            refreshStatus();
          }
        }
      } catch {
        if (isMounted) {
          setApiKeyConfigured(false);
        }
      }
    }

    initialCheck();

    return () => {
      isMounted = false;
    };
  }, [refreshStatus]);

  // Dynamic content-driven native window layout resizing & positioning anchor
  useEffect(() => {
    if (isSettingsOpen) {
      setWindowLayoutState("SETTINGS");
    } else if (isChatOpen) {
      setWindowLayoutState("CHAT");
    } else if (bubbleMessage) {
      // Content-driven height for ambient/proactive message bubble:
      // Bubble card height + avatar gap (96px) + safe margins (24px)
      const targetHeight = Math.min(360, Math.max(160, bubbleCardHeight + 120));
      setWindowLayoutState("BUBBLE", 320, targetHeight);
    } else {
      setWindowLayoutState("CLOSED");
    }
  }, [isChatOpen, isSettingsOpen, bubbleMessage, bubbleCardHeight]);

  // Update lastChatTime when messages change
  useEffect(() => {
    if (messages.length > 0) {
      lastChatTimeRef.current = Date.now();
    }
  }, [messages]);

  // ── Voice Input Handler [Feature 3.2, Feature 3.3] ──────────────────────────
  const handleTriggerVoice = useCallback(async () => {
    try {
      setAvatarState("listening");
      const compName = await getCompanionName().catch(() => companionName);
      setBubbleSender(compName);
      setBubbleMessage("Listening... 🎙️");

      const res = await startVoiceInput();
      lastChatTimeRef.current = Date.now();
      if (res.response) {
        setBubbleSender(compName);
        setBubbleMessage(res.response);
        const emotion = detectSentiment(res.response);
        setAvatarState(emotion);
        setMessages((prev) => [
          ...prev,
          {
            id: crypto.randomUUID(),
            role: "user",
            content: res.transcription,
            timestamp: Date.now(),
          },
          {
            id: crypto.randomUUID(),
            role: "assistant",
            content: res.response,
            timestamp: Date.now(),
          },
        ]);
      } else {
        setAvatarState("happy");
      }
      setTimeout(() => setAvatarState("idle"), 4000);
    } catch (e: unknown) {
      setAvatarState("concerned");
      const errMsg = e instanceof Error ? e.message : String(e);
      setBubbleMessage(errMsg);
      setTimeout(() => setAvatarState("idle"), 5000);
    }
  }, [companionName]);

  // Listen for global shortcut Cmd+Shift+Space, TTS playback, and proactive ambient messages [Feature 3.2, Feature 3.3]
  useEffect(() => {
    const unlistenTrigger = listen("trigger-voice-input", () => {
      handleTriggerVoice();
    });
    const unlistenGlobal = listen("global-voice-trigger", () => {
      handleTriggerVoice();
    });
    const unlistenTtsStart = listen("tts-started", () => {
      setAvatarState("talking");
    });
    const unlistenTtsEnd = listen("tts-ended", () => {
      setAvatarState("idle");
    });
    const unlistenProactive = listen<{ message: string; app_name?: string }>(
      "proactive-ambient-message",
      (event) => {
        const msg = event.payload?.message;
        if (msg && msg.trim()) {
          setBubbleSender(companionName);
          setBubbleMessage(msg.trim());
          const emotion = detectSentiment(msg);
          setAvatarState(emotion);
          setTimeout(() => setAvatarState("idle"), 4500);
        }
      }
    );

    return () => {
      unlistenTrigger.then((fn) => fn());
      unlistenGlobal.then((fn) => fn());
      unlistenTtsStart.then((fn) => fn());
      unlistenTtsEnd.then((fn) => fn());
      unlistenProactive.then((fn) => fn());
    };
  }, [handleTriggerVoice, companionName]);

  // ── Ambient Message Scheduler [Step 2 & Step 3] ────────────────────────────
  useEffect(() => {
    if (!apiKeyConfigured) return;

    const scheduleNextAmbient = () => {
      // Random 8-15 minute interval
      const delay = (8 + Math.random() * 7) * 60 * 1000;

      return setTimeout(async () => {
        const now = Date.now();
        const timeSinceChat = now - lastChatTimeRef.current;

        // Only show if:
        // 1. Proactive mode is not Off
        // 2. User hasn't chatted in last 5 minutes
        // 3. Chat panel is closed
        try {
          const pMode = await getProactiveMode().catch(() => proactiveMode);
          if (!isChatOpen && timeSinceChat > 5 * 60 * 1000 && pMode !== "off") {
            let chosenMessage = "";

            // Step 3: When proactive mode is Active, call Gemini
            if (pMode === "active") {
              try {
                const aiMsg = await generateAmbientMessage();
                if (aiMsg && aiMsg.length < 100 && !aiMsg.toLowerCase().startsWith("skip")) {
                  chosenMessage = aiMsg.trim();
                }
              } catch {
                // Fallback to canned pool
              }
            }

            if (!chosenMessage) {
              const hour = new Date().getHours();
              const poolKey = hour < 12 ? "morning" : hour < 18 ? "afternoon" : "evening";
              const pool = AMBIENT_MESSAGES[poolKey];
              chosenMessage = pool[Math.floor(Math.random() * pool.length)];
            }

            if (chosenMessage) {
              const compName = await getCompanionName().catch(() => companionName);
              setBubbleSender(compName || companionName);
              setBubbleMessage(chosenMessage);
              setAvatarState("happy");
              setTimeout(() => setAvatarState("idle"), 3000);
            }
          }
        } catch {
          // Scheduler non-blocking error handling
        }

        // Schedule next
        ambientTimer.current = scheduleNextAmbient();
      }, delay);
    };

    ambientTimer.current = scheduleNextAmbient();
    return () => {
      if (ambientTimer.current) clearTimeout(ambientTimer.current);
    };
  }, [apiKeyConfigured, isChatOpen, proactiveMode, companionName, userName]);

  // Loading state while checking OS keychain on startup
  if (apiKeyConfigured === null) {
    return (
      <div
        style={{
          position: "fixed",
          inset: 0,
          background: "rgba(10,10,20,0.96)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        <div className="flex items-center gap-3">
          <div className="w-5 h-5 rounded-full border-2 border-accent border-t-transparent animate-spin" />
          <span className="text-xs text-neutral-400">Loading OpenMate...</span>
        </div>
      </div>
    );
  }

  // Onboarding — full screen opaque container
  if (!apiKeyConfigured) {
    return (
      <div style={{ position: "fixed", inset: 0, background: "rgba(10,10,20,0.98)" }}>
        <Onboarding
          onComplete={() => {
            setApiKeyConfigured(true);
            refreshStatus();
          }}
        />
      </div>
    );
  }

  // ── Main Shell — transparent overlay ────────────────────────────────────
  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "transparent",
        pointerEvents: "none",
        overflow: "hidden",
      }}
    >
      {/* Ambient Message Bubble — floats near avatar [Feature 2.1] */}
      <AnimatePresence>
        {bubbleMessage && (
          <MessageBubble
            message={bubbleMessage}
            senderName={bubbleSender}
            durationMs={6000}
            onHeightChange={(h) => setBubbleCardHeight(h)}
            onClose={() => setBubbleMessage(null)}
            onClick={() => {
              setBubbleMessage(null);
              setIsChatOpen(true);
            }}
          />
        )}
      </AnimatePresence>

      {/* Slide-in Chat Panel — only present & interactive when open */}
      <AnimatePresence>
        {isChatOpen && (
          <ChatPanel
            isOpen={isChatOpen}
            onClose={() => setIsChatOpen(false)}
            onOpenSettings={() => setIsSettingsOpen(true)}
            currentMode={currentMode}
            onModeChange={setCurrentMode}
            setAvatarState={setAvatarState}
            messages={messages}
            setMessages={setMessages}
          />
        )}
      </AnimatePresence>

      {/* Interactive Avatar Overlay — tightly bounded 72x72 circle */}
      <Avatar
        state={avatarState}
        onToggleChat={() => setIsChatOpen((prev) => !prev)}
        isChatOpen={isChatOpen}
        isMicAllowed={isMicAllowed}
        onStartVoice={handleTriggerVoice}
      />

      {/* Settings Modal — only present & interactive when open */}
      <AnimatePresence>
        {isSettingsOpen && (
          <Settings
            isOpen={isSettingsOpen}
            onClose={() => {
              setIsSettingsOpen(false);
              refreshStatus();
            }}
            onKeySaved={() => {
              setApiKeyConfigured(true);
              refreshStatus();
            }}
            onKeyRemoved={() => {
              setApiKeyConfigured(false);
            }}
          />
        )}
      </AnimatePresence>
    </div>
  );
}
