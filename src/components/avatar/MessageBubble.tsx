/**
 * OpenMate Message Bubble Component
 *
 * Ambient speech bubble that floats above/near the cat avatar.
 * Displays proactive thoughts, ambient greetings, voice transcriptions,
 * and quick AI responses even when the full chat panel is closed.
 * Dynamically measures content height to resize the native window.
 * [Feature 2.1, Feature 3.3]
 */

import { useEffect, useLayoutEffect, useRef } from "react";
import { motion } from "framer-motion";

interface MessageBubbleProps {
  message: string;
  senderName?: string;
  durationMs?: number;
  onClose: () => void;
  onClick: () => void;
  onHeightChange?: (height: number) => void;
}

export default function MessageBubble({
  message,
  senderName,
  durationMs = 6000,
  onClose,
  onClick,
  onHeightChange,
}: MessageBubbleProps) {
  const cardRef = useRef<HTMLDivElement>(null);

  // Auto-dismiss timer
  useEffect(() => {
    if (!message) return;
    const timer = setTimeout(() => {
      onClose();
    }, durationMs);

    return () => clearTimeout(timer);
  }, [message, durationMs, onClose]);

  // Measure rendered bubble height and notify parent for native window expansion
  useLayoutEffect(() => {
    if (cardRef.current && onHeightChange) {
      const height = cardRef.current.offsetHeight;
      onHeightChange(height);
    }
  }, [message, senderName, onHeightChange]);

  if (!message) return null;

  return (
    <motion.div
      initial={{ opacity: 0, y: 12, scale: 0.92 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: -8, scale: 0.92 }}
      transition={{ duration: 0.22, ease: "easeOut" }}
      onClick={onClick}
      className="absolute bottom-24 right-3 z-50 w-[290px] cursor-pointer select-none group"
      style={{ pointerEvents: "auto" }}
    >
      {/* Speech bubble card */}
      <div
        ref={cardRef}
        className="relative bg-surface-elevated/95 backdrop-blur-md border border-surface-border text-neutral-100 rounded-2xl px-4 py-3 shadow-2xl hover:border-accent/50 transition-all duration-200"
      >
        {senderName && (
          <div className="text-[10px] font-semibold tracking-wider uppercase text-accent-light mb-1">
            {senderName}
          </div>
        )}
        <div className="text-xs text-neutral-200 leading-relaxed whitespace-pre-wrap break-words max-h-[180px] overflow-y-auto pr-1">
          {message}
        </div>

        {/* Bubble tail pointing to the avatar below */}
        <div
          className="absolute -bottom-2 right-8 w-3.5 h-3.5 bg-surface-elevated border-r border-b border-surface-border rotate-45"
          style={{ clipPath: "polygon(0 0, 100% 0, 100% 100%)" }}
        />
      </div>
    </motion.div>
  );
}
