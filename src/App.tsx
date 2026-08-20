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
  generateAmbientMessage,
  startVoiceInput,
} from "./hooks/useIpc";
import type { AvatarState, CompanionMode, ChatMessage } from "./types";

import Onboarding from "./components/onboarding/Onboarding";
import Avatar from "./components/avatar/Avatar";
import MessageBubble from "./components/avatar/MessageBubble";
import ChatPanel from "./components/chat/ChatPanel";
import Settings from "./components/settings/Settings";
import { initializeWindowLayout, setWindowLayoutState } from "./utils/windowLayout";

// Ambient messages pools by time of day [Feature 2.2]
const MORNING_POOL = [
  "Good morning! Ready to conquer the day? *stretches*",
  "Psst... I made you an imaginary coffee ☕",
  "Morning! What are we working on today?",
];

const AFTERNOON_POOL = [
  "How's it going? Need a break? *yawns*",
  "You've been working hard. I noticed. 👀",
  "Anything I can help with?",
];

const EVENING_POOL = [
  "Still here! Don't forget to rest.",
  "What did you get done today? Tell me everything.",
  "I'm bored. Talk to me? *paws at screen*",
];

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

  // Ambient Message Bubble state [Feature 2.1]
  const [bubbleMessage, setBubbleMessage] = useState<string | null>(null);
  const [bubbleSender, setBubbleSender] = useState<string>("OpenMate");
  const [companionName, setCompanionName] = useState<string>("OpenMate");
  const [bubbleCardHeight, setBubbleCardHeight] = useState<number>(70);

  // Track chat activity for idle detection [Feature 2.2]
  const lastChatTimeRef = useRef<number>(Date.now());
  const shownAmbientRef = useRef<Set<string>>(new Set());

  // Check permissions & configuration
  const refreshStatus = useCallback(async () => {
    try {
      const perms = await getPermissions();
      const mic = perms.find((p) => p.capability === "microphone");
      setIsMicAllowed(mic?.state === "allow");

      const name = await getCompanionName();
      setCompanionName(name || "OpenMate");
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
      const compName = await getCompanionName().catch(() => "OpenMate");
      setBubbleSender(compName);
      setBubbleMessage("Listening... 🎙️");

      const res = await startVoiceInput();
      setAvatarState("happy");
      lastChatTimeRef.current = Date.now();

      // Show user transcription
      setBubbleSender("You");
      setBubbleMessage(`"${res.transcription}"`);

      // After 2 seconds, show companion response
      setTimeout(() => {
        setBubbleSender(compName);
        if (res.response.length <= 100) {
          setBubbleMessage(res.response);
        } else {
          setBubbleMessage(res.response.slice(0, 95) + "...");
          setIsChatOpen(true);
        }

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

        setTimeout(() => setAvatarState("idle"), 4000);
      }, 2000);
    } catch (err) {
      setAvatarState("concerned");
      setBubbleSender(companionName);
      setBubbleMessage(`Voice input: ${String(err)}`);
      setTimeout(() => setAvatarState("idle"), 3500);
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
          setAvatarState("happy");
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

  // ── Ambient Message Scheduler [Feature 2.2, Feature 2.3] ────────────────────
  useEffect(() => {
    if (!apiKeyConfigured) return;

    // Check every 60 seconds whether it's time to trigger an ambient bubble
    const interval = setInterval(async () => {
      const now = Date.now();
      const idleMinutes = (now - lastChatTimeRef.current) / (1000 * 60);

      // Rule: only show if user has been idle from chat for at least 5 minutes
      if (idleMinutes < 5) return;

      try {
        const pMode = await getProactiveMode();
        if (pMode === "off") return;

        const currentHour = new Date().getHours();
        let pool = EVENING_POOL;
        if (currentHour >= 6 && currentHour < 12) {
          pool = MORNING_POOL;
        } else if (currentHour >= 12 && currentHour < 18) {
          pool = AFTERNOON_POOL;
        }

        let chosenMessage = "";

        // If proactive mode is Active, try Gemini generation first [Feature 2.3]
        if (pMode === "active") {
          try {
            const aiMsg = await generateAmbientMessage();
            if (aiMsg && aiMsg.length > 3) {
              chosenMessage = aiMsg;
            }
          } catch {
            // fallback to canned pool
          }
        }

        if (!chosenMessage) {
          const available = pool.filter((msg) => !shownAmbientRef.current.has(msg));
          const list = available.length > 0 ? available : pool;
          chosenMessage = list[Math.floor(Math.random() * list.length)];
          shownAmbientRef.current.add(chosenMessage);
        }

        if (chosenMessage) {
          const compName = await getCompanionName().catch(() => "OpenMate");
          setBubbleSender(compName);
          setBubbleMessage(chosenMessage);
          setAvatarState("happy");
          setTimeout(() => setAvatarState("idle"), 4000);
        }
      } catch {
        // Scheduler non-blocking error handling
      }
    }, 90000); // Check every 90 seconds (idle >= 5 min guard applies)

    return () => clearInterval(interval);
  }, [apiKeyConfigured]);

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
