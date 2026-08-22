/**
 * OpenMate Cat Avatar — 2D sprite desktop companion
 *
 * Renders an expressive cat-face mascot with state-driven SVG animations.
 * Each of the 6 states has a meaningfully distinct cat expression.
 * The window is a transparent overlay — the avatar must NOT have any
 * background behind it (the container background is transparent).
 *
 * [ADR-003, DR-016, Fix-2]
 */

import { useState, useEffect, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import type { AvatarState } from "../../types";
import { getActiveAvatar, getAvatarImage } from "../../hooks/useIpc";
import { startNativeDrag } from "../../utils/windowLayout";

interface AvatarProps {
  state: AvatarState;
  onToggleChat: () => void;
  isChatOpen: boolean;
  isMicAllowed?: boolean;
  onStartVoice?: () => void;
}

// ── Shared cat base parts ────────────────────────────────────────────────────

/** Left ear */
function EarLeft({ color = "#6366f1" }: { color?: string }) {
  return (
    <g>
      {/* Outer ear */}
      <polygon points="12,8 18,22 6,22" fill={color} />
      {/* Inner ear (pink) */}
      <polygon points="12,11 16,21 8,21" fill="#f472b6" opacity="0.6" />
    </g>
  );
}

/** Right ear */
function EarRight({ color = "#6366f1" }: { color?: string }) {
  return (
    <g>
      {/* Outer ear */}
      <polygon points="52,8 58,22 46,22" fill={color} />
      {/* Inner ear (pink) */}
      <polygon points="52,11 56,21 48,21" fill="#f472b6" opacity="0.6" />
    </g>
  );
}

/** Cat nose triangle */
function Nose() {
  return <polygon points="32,36 29,40 35,40" fill="#f472b6" />;
}

/** Six whiskers: 3 on each side */
function Whiskers() {
  return (
    <g stroke="rgba(255,255,255,0.55)" strokeWidth="0.8" strokeLinecap="round">
      {/* Left whiskers */}
      <line x1="28" y1="37" x2="10" y2="34" />
      <line x1="28" y1="39" x2="10" y2="39" />
      <line x1="28" y1="41" x2="10" y2="44" />
      {/* Right whiskers */}
      <line x1="36" y1="37" x2="54" y2="34" />
      <line x1="36" y1="39" x2="54" y2="39" />
      <line x1="36" y1="41" x2="54" y2="44" />
    </g>
  );
}

// ── State-specific expressions ───────────────────────────────────────────────

/** IDLE — Normal open eyes, gentle smile, content */
function IdleFace() {
  return (
    <>
      {/* Eyes — normal circles with blink */}
      <motion.ellipse
        cx="24" cy="33" rx="4" ry="4"
        fill="white"
        animate={{ scaleY: [1, 1, 0.08, 1, 1] }}
        transition={{ duration: 5, repeat: Infinity, times: [0, 0.88, 0.92, 0.96, 1] }}
        style={{ transformOrigin: "24px 33px" }}
      />
      <circle cx="24" cy="34" r="2" fill="#1e1b4b" />
      <circle cx="23" cy="33" r="0.8" fill="white" />

      <motion.ellipse
        cx="40" cy="33" rx="4" ry="4"
        fill="white"
        animate={{ scaleY: [1, 1, 0.08, 1, 1] }}
        transition={{ duration: 5, repeat: Infinity, times: [0, 0.88, 0.92, 0.96, 1] }}
        style={{ transformOrigin: "40px 33px" }}
      />
      <circle cx="40" cy="34" r="2" fill="#1e1b4b" />
      <circle cx="39" cy="33" r="0.8" fill="white" />

      <Nose />
      <Whiskers />
      {/* Gentle content smile */}
      <path d="M28,43 Q32,47 36,43" stroke="white" strokeWidth="1.2" fill="none" strokeLinecap="round" />
    </>
  );
}

/** THINKING — Eyes shifted up-right, small 'o' mouth, thought dots */
function ThinkingFace() {
  return (
    <>
      {/* Eyes looking up-right */}
      <ellipse cx="24" cy="33" rx="4" ry="4" fill="white" />
      <circle cx="25" cy="31" r="2" fill="#1e1b4b" />
      <circle cx="24" cy="31" r="0.8" fill="white" />

      <ellipse cx="40" cy="33" rx="4" ry="4" fill="white" />
      <circle cx="41" cy="31" r="2" fill="#1e1b4b" />
      <circle cx="40" cy="31" r="0.8" fill="white" />

      <Nose />
      <Whiskers />
      {/* Small 'o' mouth */}
      <ellipse cx="32" cy="44" rx="2" ry="2.5" stroke="white" strokeWidth="1.2" fill="none" />

      {/* Animated thought bubble dots above head */}
      <motion.circle cx="48" cy="14" r="1.5" fill="#a5b4fc"
        animate={{ opacity: [0, 1, 0], y: [0, -3, 0] }}
        transition={{ duration: 1.2, repeat: Infinity, delay: 0 }} />
      <motion.circle cx="52" cy="10" r="2" fill="#818cf8"
        animate={{ opacity: [0, 1, 0], y: [0, -3, 0] }}
        transition={{ duration: 1.2, repeat: Infinity, delay: 0.3 }} />
      <motion.circle cx="57" cy="6" r="2.5" fill="#6366f1"
        animate={{ opacity: [0, 1, 0], y: [0, -4, 0] }}
        transition={{ duration: 1.2, repeat: Infinity, delay: 0.6 }} />
    </>
  );
}

/** TALKING — Happy closed-arch eyes (^ ^), animated open mouth */
function TalkingFace() {
  return (
    <>
      {/* Happy arch eyes ^ ^ */}
      <path d="M20,32 Q24,27 28,32" stroke="white" strokeWidth="2" fill="none" strokeLinecap="round" />
      <path d="M36,32 Q40,27 44,32" stroke="white" strokeWidth="2" fill="none" strokeLinecap="round" />

      <Nose />
      <Whiskers />
      {/* Animated speaking oval mouth */}
      <motion.ellipse
        cx="32" cy="44" rx="4" ry="3"
        fill="#1e1b4b"
        stroke="white"
        strokeWidth="1.2"
        animate={{ ry: [1.5, 4, 1.5] }}
        transition={{ duration: 0.32, repeat: Infinity, ease: "easeInOut" }}
      />
    </>
  );
}

/** HAPPY — Closed happy arcs (≧◡≦), wide smile, pink blush */
function HappyFace() {
  return (
    <>
      {/* Closed happy ≧◡≦ eyes */}
      <path d="M19,34 Q24,29 29,34" stroke="white" strokeWidth="2.2" fill="none" strokeLinecap="round" />
      <path d="M35,34 Q40,29 45,34" stroke="white" strokeWidth="2.2" fill="none" strokeLinecap="round" />

      {/* Blush circles */}
      <ellipse cx="18" cy="40" rx="5" ry="3" fill="#f472b6" opacity="0.4" />
      <ellipse cx="46" cy="40" rx="5" ry="3" fill="#f472b6" opacity="0.4" />

      <Nose />
      <Whiskers />
      {/* Wide happy smile */}
      <path d="M24,44 Q32,52 40,44" stroke="white" strokeWidth="1.8" fill="none" strokeLinecap="round" />
    </>
  );
}

/** CONCERNED — Asymmetric brow, worried eyes, slight frown */
function ConcernedFace() {
  return (
    <>
      {/* Eyebrows — left raised, right normal */}
      <path d="M20,25 L28,28" stroke="white" strokeWidth="1.5" strokeLinecap="round" />
      <path d="M36,27 L44,24" stroke="white" strokeWidth="1.5" strokeLinecap="round" />

      {/* Slightly worried eyes */}
      <ellipse cx="24" cy="33" rx="3.5" ry="3.5" fill="white" />
      <circle cx="24" cy="33" r="1.8" fill="#1e1b4b" />

      <ellipse cx="40" cy="33" rx="3.5" ry="3.5" fill="white" />
      <circle cx="40" cy="33" r="1.8" fill="#1e1b4b" />

      <Nose />
      <Whiskers />
      {/* Slight frown */}
      <path d="M26,45 Q32,41 38,45" stroke="white" strokeWidth="1.2" fill="none" strokeLinecap="round" />
    </>
  );
}

/** LISTENING — Wide attentive eyes, pulsing sound-wave arcs on sides of head */
function ListeningFace() {
  return (
    <>
      {/* Wide attentive eyes */}
      <ellipse cx="24" cy="33" rx="5" ry="5" fill="white" />
      <circle cx="24" cy="33" r="2.5" fill="#1e1b4b" />
      <circle cx="22.5" cy="31.5" r="1" fill="white" />

      <ellipse cx="40" cy="33" rx="5" ry="5" fill="white" />
      <circle cx="40" cy="33" r="2.5" fill="#1e1b4b" />
      <circle cx="38.5" cy="31.5" r="1" fill="white" />

      <Nose />
      <Whiskers />
      {/* Neutral mouth */}
      <path d="M28,44 Q32,46 36,44" stroke="white" strokeWidth="1.2" fill="none" strokeLinecap="round" />

      {/* Left sound-wave arcs */}
      <motion.path d="M5,28 Q2,33 5,38" stroke="#a5b4fc" strokeWidth="1.5" fill="none" strokeLinecap="round"
        animate={{ opacity: [0.3, 1, 0.3], scaleX: [0.8, 1.2, 0.8] }}
        transition={{ duration: 0.9, repeat: Infinity }}
        style={{ transformOrigin: "5px 33px" }} />
      <motion.path d="M2,24 Q-2,33 2,42" stroke="#818cf8" strokeWidth="1.5" fill="none" strokeLinecap="round"
        animate={{ opacity: [0.2, 0.8, 0.2], scaleX: [0.7, 1.3, 0.7] }}
        transition={{ duration: 0.9, repeat: Infinity, delay: 0.15 }}
        style={{ transformOrigin: "2px 33px" }} />

      {/* Right sound-wave arcs */}
      <motion.path d="M59,28 Q62,33 59,38" stroke="#a5b4fc" strokeWidth="1.5" fill="none" strokeLinecap="round"
        animate={{ opacity: [0.3, 1, 0.3], scaleX: [0.8, 1.2, 0.8] }}
        transition={{ duration: 0.9, repeat: Infinity }}
        style={{ transformOrigin: "59px 33px" }} />
      <motion.path d="M62,24 Q66,33 62,42" stroke="#818cf8" strokeWidth="1.5" fill="none" strokeLinecap="round"
        animate={{ opacity: [0.2, 0.8, 0.2], scaleX: [0.7, 1.3, 0.7] }}
        transition={{ duration: 0.9, repeat: Infinity, delay: 0.15 }}
        style={{ transformOrigin: "62px 33px" }} />
    </>
  );
}

// ── Main Avatar component ────────────────────────────────────────────────────

export default function Avatar({
  state,
  onToggleChat,
  isChatOpen,
  isMicAllowed,
  onStartVoice,
}: AvatarProps) {
  const currentState = state;
  const isHappy = currentState === "happy";
  const isConcerned = currentState === "concerned";
  const headColor = isConcerned ? "#4c0519" : "#1e1b4b";
  const borderColor = isConcerned ? "#ef4444" : "#6366f1";
  const earColor = isConcerned ? "#7f1d1d" : "#6366f1";

  const [avatarMode, setAvatarMode] = useState<"builtin" | "package">("builtin");
  const [packageImages, setPackageImages] = useState<Record<string, string>>({});

  const loadActiveAvatar = useCallback(async () => {
    try {
      const activeName = await getActiveAvatar();
      if (!activeName || activeName === "default" || activeName === "builtin") {
        setAvatarMode("builtin");
        setPackageImages({});
        return;
      }

      // Load all 6 states for custom package
      const states: AvatarState[] = ["idle", "thinking", "talking", "happy", "concerned", "listening"];
      const images: Record<string, string> = {};
      for (const st of states) {
        try {
          const rawBase64 = await getAvatarImage(activeName, st);
          const dataUrl = rawBase64.startsWith("data:")
            ? rawBase64
            : `data:image/png;base64,${rawBase64}`;
          images[st] = dataUrl;
        } catch (err) {
          console.warn(`Failed to load avatar state image ${st}:`, err);
        }
      }

      if (Object.keys(images).length === 6) {
        setPackageImages(images);
        setAvatarMode("package");
      } else {
        setAvatarMode("builtin");
      }
    } catch {
      setAvatarMode("builtin");
    }
  }, []);

  useEffect(() => {
    loadActiveAvatar();

    const handleAvatarChange = () => {
      loadActiveAvatar();
    };

    window.addEventListener("openmate-avatar-changed", handleAvatarChange);
    return () => {
      window.removeEventListener("openmate-avatar-changed", handleAvatarChange);
    };
  }, [loadActiveAvatar]);

  return (
    <motion.div
      data-tauri-drag-region
      onPointerDown={(e) => {
        if (e.button === 0) {
          startNativeDrag();
        }
      }}
      whileHover={{ scale: 1.07 }}
      whileTap={{ scale: 0.93 }}
      // Floating animation per state
      animate={
        isHappy
          ? { y: [0, -10, 0] }
          : currentState === "idle"
          ? { y: [0, -6, 0] }
          : { y: 0 }
      }
      transition={{
        duration: isHappy ? 0.5 : 2.5,
        repeat: Infinity,
        ease: "easeInOut",
      }}
      className="absolute bottom-3 right-3 z-50 cursor-grab active:cursor-grabbing select-none w-[72px] h-[72px] rounded-full"
      style={{ pointerEvents: "auto" }}
    >
      {/* Ambient glow ring */}
      <motion.div
        data-tauri-drag-region
        className="absolute inset-0 rounded-full"
        animate={{
          boxShadow:
            currentState === "listening"
              ? [
                  "0 0 12px 4px rgba(99,102,241,0.4)",
                  "0 0 28px 10px rgba(99,102,241,0.85)",
                  "0 0 12px 4px rgba(99,102,241,0.4)",
                ]
              : currentState === "thinking"
              ? "0 0 22px 6px rgba(129,140,248,0.55)"
              : currentState === "talking"
              ? "0 0 18px 4px rgba(99,102,241,0.45)"
              : currentState === "concerned"
              ? "0 0 18px 4px rgba(239,68,68,0.5)"
              : isHappy
              ? "0 0 22px 6px rgba(34,197,94,0.5)"
              : "0 0 12px 2px rgba(99,102,241,0.2)",
        }}
        transition={{
          duration: currentState === "listening" ? 1.1 : 0.4,
          repeat: currentState === "listening" ? Infinity : 0,
          ease: "easeInOut",
        }}
      />

      <button
        data-tauri-drag-region
        type="button"
        onPointerDown={(e) => {
          if (e.button === 0) {
            startNativeDrag();
          }
        }}
        onClick={onToggleChat}
        title={isChatOpen ? "Close companion chat" : "Open companion chat"}
        aria-label={isChatOpen ? "Close companion chat" : "Open companion chat"}
        className="relative flex items-center justify-center focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent cursor-grab active:cursor-grabbing"
        style={{
          width: 72,
          height: 72,
          borderRadius: "50%",
          border: `2.5px solid ${borderColor}`,
          background: headColor,
          overflow: "visible",
          boxShadow: "0 8px 32px rgba(0,0,0,0.55)",
        }}
      >
        {avatarMode === "builtin" ? (
          /* Cat SVG canvas — viewBox spans 0 0 64 64, ears extend above */
          <AnimatePresence mode="wait">
            <motion.svg
              key={currentState}
              viewBox="0 0 64 64"
              width={72}
              height={72}
              initial={{ opacity: 0, scale: 0.88 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.88 }}
              transition={{ duration: 0.18 }}
              overflow="visible"
              style={{ display: "block" }}
            >
              {/* Head circle */}
              <circle cx="32" cy="38" r="22" fill={headColor} />

              {/* Ears */}
              <g transform="translate(2, 2)">
                <EarLeft color={earColor} />
                <EarRight color={earColor} />
              </g>

              {/* Face expression */}
              {currentState === "idle" && <IdleFace />}
              {currentState === "thinking" && <ThinkingFace />}
              {currentState === "talking" && <TalkingFace />}
              {currentState === "happy" && <HappyFace />}
              {currentState === "concerned" && <ConcernedFace />}
              {currentState === "listening" && <ListeningFace />}
            </motion.svg>
          </AnimatePresence>
        ) : (
          <AnimatePresence mode="wait">
            <motion.img
              key={currentState}
              src={packageImages[currentState] || packageImages["idle"]}
              alt={`Avatar ${currentState}`}
              initial={{ opacity: 0, scale: 0.88 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.88 }}
              transition={{ duration: 0.18 }}
              className="w-full h-full object-contain rounded-full select-none pointer-events-none p-1"
              draggable={false}
            />
          </AnimatePresence>
        )}

        {/* Status dot */}
        <span
          className={`absolute bottom-1 right-1 w-2.5 h-2.5 rounded-full border-2 ${
            currentState === "thinking"
              ? "bg-accent-light animate-pulse border-darkBg"
              : currentState === "talking"
              ? "bg-accent animate-ping border-darkBg"
              : currentState === "listening"
              ? "bg-accent animate-ping border-darkBg"
              : currentState === "concerned"
              ? "bg-red-500 border-darkBg"
              : currentState === "happy"
              ? "bg-green-400 border-darkBg"
              : "bg-green-400 border-darkBg"
          }`}
        />
      </button>

      {/* Quick-trigger Voice Mic Indicator [Feature 3.1] */}
      {isMicAllowed && (
        <motion.button
          type="button"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            if (onStartVoice) onStartVoice();
          }}
          title="Quick voice input (or press Cmd+Shift+Space)"
          aria-label="Quick voice input"
          whileHover={{ scale: 1.15 }}
          whileTap={{ scale: 0.9 }}
          className="absolute -top-1 -left-1 p-1.5 rounded-full bg-surface-elevated/90 border border-accent/60 text-accent-light shadow-md hover:bg-accent hover:text-white transition-colors"
          style={{ pointerEvents: "all" }}
        >
          {/* Animated sound wave bars or mic icon */}
          <svg viewBox="0 0 24 24" className="w-3.5 h-3.5 fill-none stroke-current" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z" />
            <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
            <line x1="12" y1="19" x2="12" y2="22" />
          </svg>
        </motion.button>
      )}
    </motion.div>
  );
}
