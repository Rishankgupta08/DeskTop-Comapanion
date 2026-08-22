/**
 * OpenMate Chat Panel
 *
 * Slide-in panel featuring mode selector tabs, auto-scrolling message history,
 * multi-line input handling, async request orchestration with avatar state feedback,
 * interactive tool confirmation cards for filesystem writes [DR-018, DR-033],
 * microphone voice input integration [DR-005, DR-012], lightweight markdown rendering,
 * message copy button on hover, and message timestamps.
 * [SRS FR-020, DR-005, DR-015, DR-029, DR-032, DR-033]
 */

import { useState, useRef, useEffect, useMemo } from "react";
import { motion } from "framer-motion";
import {
  sendMessage,
  requestScreenContext,
  setMode as setModeIpc,
  executeTool,
  setPermission,
  startVoiceInput,
  getCompanionName,
  getUserName,
  getMemories,
} from "../../hooks/useIpc";
import { modeLoader } from "../../engine/mode-loader";
import type { CompanionMode, ChatMessage, AvatarState, ModeManifest } from "../../types";

interface ChatPanelProps {
  isOpen: boolean;
  onClose: () => void;
  onOpenSettings?: () => void;
  currentMode: CompanionMode;
  onModeChange: (mode: CompanionMode) => void;
  setAvatarState: (state: AvatarState) => void;
  messages: ChatMessage[];
  setMessages: React.Dispatch<React.SetStateAction<ChatMessage[]>>;
}

const BUILTIN_OPTIONS: { id: CompanionMode; label: string; description: string; icon?: string }[] = [
  { id: "play", label: "Play", description: "Let's hang out" },
  { id: "coder", label: "Coder", description: "Help with your code" },
  { id: "assistant", label: "Assistant", description: "Get things done" },
  { id: "personal_friend", label: "Friend", description: "Just talk" },
];

interface PendingWrite {
  messageId: string;
  path: string;
  content: string;
}

interface FileConflict {
  messageId: string;
  path: string;
  content: string;
  errorMessage: string;
}

export function detectSentiment(response: string): AvatarState {
  const lower = response.toLowerCase();
  if (
    lower.includes("sorry") ||
    lower.includes("oh no") ||
    (lower.includes("that") && lower.includes("hard")) ||
    lower.includes("sad") ||
    lower.includes("hurt") ||
    lower.includes("heavy") ||
    lower.includes("rough") ||
    lower.includes("bad day")
  ) {
    return "concerned";
  }
  if (
    lower.includes("!") &&
    (lower.includes("great") ||
      lower.includes("amazing") ||
      lower.includes("yes") ||
      lower.includes("yay") ||
      lower.includes("purr") ||
      lower.includes("love") ||
      lower.includes("awesome") ||
      lower.includes("happy"))
  ) {
    return "happy";
  }
  if (
    lower.includes("*thinks*") ||
    lower.includes("hmm") ||
    lower.includes("let me") ||
    lower.includes("wonder") ||
    lower.includes("puzzl")
  ) {
    return "thinking";
  }
  return "talking";
}

function formatTimestamp(timestamp?: number): string {
  if (!timestamp) return "";
  const d = new Date(timestamp);
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
}

function renderInlineMarkdown(text: string) {
  const tokens = text.split(/(\*\*.*?\*\*|`.*?`)/g);

  return tokens.map((token, idx) => {
    if (token.startsWith("**") && token.endsWith("**") && token.length >= 4) {
      return (
        <strong key={idx} className="font-semibold text-white">
          {token.slice(2, -2)}
        </strong>
      );
    }
    if (token.startsWith("`") && token.endsWith("`") && token.length >= 2) {
      return (
        <code
          key={idx}
          className="px-1 py-0.5 bg-darkBg/90 border border-surface-border rounded text-[11px] font-mono text-accent-light"
        >
          {token.slice(1, -1)}
        </code>
      );
    }
    return token;
  });
}

function parseMarkdown(text: string) {
  const parts = text.split(/(```[\s\S]*?```)/g);

  return parts.map((part, index) => {
    if (part.startsWith("```") && part.endsWith("```")) {
      const firstLineEnd = part.indexOf("\n");
      const content =
        firstLineEnd !== -1 ? part.slice(firstLineEnd + 1, -3) : part.slice(3, -3);
      return (
        <pre
          key={index}
          className="my-1.5 p-2.5 bg-darkBg border border-surface-border rounded-xl overflow-x-auto text-[11px] font-mono text-neutral-200"
        >
          <code>{content}</code>
        </pre>
      );
    }

    const lines = part.split("\n");
    return (
      <div key={index} className="space-y-1">
        {lines.map((line, lineIdx) => {
          if (line.trim().startsWith("- ") || line.trim().startsWith("* ")) {
            const itemText = line.trim().replace(/^[-*]\s+/, "");
            return (
              <div key={lineIdx} className="flex items-start gap-1.5 pl-1">
                <span className="text-accent select-none">•</span>
                <span>{renderInlineMarkdown(itemText)}</span>
              </div>
            );
          }
          return line.trim() === "" ? (
            <div key={lineIdx} className="h-1" />
          ) : (
            <p key={lineIdx}>{renderInlineMarkdown(line)}</p>
          );
        })}
      </div>
    );
  });
}

export default function ChatPanel({
  isOpen,
  onClose,
  onOpenSettings,
  currentMode,
  onModeChange,
  setAvatarState,
  messages,
  setMessages,
}: ChatPanelProps) {
  const [inputText, setInputText] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [isRecording, setIsRecording] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [pendingWrites, setPendingWrites] = useState<Record<string, PendingWrite>>({});
  const [resolvedWrites, setResolvedWrites] = useState<Record<string, "allowed" | "denied">>({});
  const [fileConflicts, setFileConflicts] = useState<Record<string, FileConflict>>({});
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [extensionManifests, setExtensionManifests] = useState<ModeManifest[]>([]);

  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Load extension modes on mount
  useEffect(() => {
    modeLoader
      .scanExtensions()
      .then((exts) => setExtensionManifests(exts))
      .catch((err) => console.warn("Failed to scan mode extensions:", err));
  }, []);

  // Compute merged mode options (built-in 4 first, then extensions)
  const allModeOptions = useMemo(() => {
    const extOptions = extensionManifests.map((m) => ({
      id: m.id,
      label: m.name,
      description: m.description,
      icon: m.icon,
    }));
    return [...BUILTIN_OPTIONS, ...extOptions];
  }, [extensionManifests]);

  const currentModeInfo = useMemo(() => {
    return allModeOptions.find((m) => m.id === currentMode) || BUILTIN_OPTIONS[2];
  }, [allModeOptions, currentMode]);

  // Auto-scroll to newest message
  useEffect(() => {
    if (isOpen) {
      messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [messages, isOpen, isLoading]);

  const handleCopy = (id: string, text: string) => {
    navigator.clipboard.writeText(text);
    setCopiedId(id);
    setTimeout(() => setCopiedId(null), 2000);
  };

  const handleModeSelect = async (mode: CompanionMode) => {
    if (mode === currentMode) return;
    onModeChange(mode);
    try {
      await setModeIpc(mode as any);
    } catch {
      // Non-blocking mode switch
    }
  };

  const parsePendingWrite = (messageId: string, content: string) => {
    const writeMatch = content.match(/\[WRITE_FILE:\s*([^\]]+)\]/);
    if (writeMatch) {
      const rawPath = writeMatch[1].trim();
      let fileContent = "";
      const codeBlockMatch = content.match(/```(?:\w+)?\n([\s\S]*?)```/);
      if (codeBlockMatch) {
        fileContent = codeBlockMatch[1];
      } else {
        const parts = rawPath.split("|");
        if (parts.length > 1) {
          fileContent = parts.slice(1).join("|").trim();
        }
      }

      const filePath = rawPath.split("|")[0].trim();
      setPendingWrites((prev) => ({
        ...prev,
        [messageId]: {
          messageId,
          path: filePath,
          content: fileContent,
        },
      }));
    }
  };

  const handleAllowWrite = async (messageId: string, alwaysAllowSession = false) => {
    const write = pendingWrites[messageId];
    if (!write) return;

    try {
      if (alwaysAllowSession) {
        await setPermission("filesystem_write", "allow");
      }

      const res = await executeTool("write_file", {
        path: write.path,
        content: write.content,
      });

      setResolvedWrites((prev) => ({ ...prev, [messageId]: "allowed" }));

      const confirmMessage: ChatMessage = {
        id: crypto.randomUUID(),
        role: "assistant",
        content: res.output || `✅ Successfully wrote file: \`${write.path}\``,
        timestamp: Date.now(),
      };
      setMessages((prev) => [...prev, confirmMessage]);
    } catch (err) {
      const errMsg = typeof err === "string" ? err : "Failed to execute file write.";
      if (errMsg.toLowerCase().includes("already exists")) {
        // Show interactive conflict card
        setFileConflicts((prev) => ({
          ...prev,
          [messageId]: {
            messageId,
            path: write.path,
            content: write.content,
            errorMessage: errMsg,
          },
        }));
      } else {
        setResolvedWrites((prev) => ({ ...prev, [messageId]: "allowed" }));
        const errResponse: ChatMessage = {
          id: crypto.randomUUID(),
          role: "assistant",
          content: `❌ Write error: ${errMsg}`,
          timestamp: Date.now(),
        };
        setMessages((prev) => [...prev, errResponse]);
      }
    }
  };

  const handleOverwrite = async (messageId: string) => {
    const conflict = fileConflicts[messageId];
    if (!conflict) return;

    try {
      setFileConflicts((prev) => {
        const next = { ...prev };
        delete next[messageId];
        return next;
      });
      setResolvedWrites((prev) => ({ ...prev, [messageId]: "allowed" }));

      const res = await executeTool("write_file", {
        path: conflict.path,
        content: conflict.content,
        overwrite: "true",
      });

      const confirmMessage: ChatMessage = {
        id: crypto.randomUUID(),
        role: "assistant",
        content: res.output || `✅ Successfully overwrote file: \`${conflict.path}\``,
        timestamp: Date.now(),
      };
      setMessages((prev) => [...prev, confirmMessage]);
    } catch (err) {
      const errMsg = typeof err === "string" ? err : "Failed to overwrite file.";
      setMessages((prev) => [
        ...prev,
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content: `❌ Overwrite error: ${errMsg}`,
          timestamp: Date.now(),
        },
      ]);
    }
  };

  const handleCreateCopy = async (messageId: string) => {
    const conflict = fileConflicts[messageId];
    if (!conflict) return;

    try {
      setFileConflicts((prev) => {
        const next = { ...prev };
        delete next[messageId];
        return next;
      });
      setResolvedWrites((prev) => ({ ...prev, [messageId]: "allowed" }));

      const res = await executeTool("write_file", {
        path: conflict.path,
        content: conflict.content,
        createCopy: "true",
      });

      const confirmMessage: ChatMessage = {
        id: crypto.randomUUID(),
        role: "assistant",
        content: res.output || `✅ Successfully created copy for: \`${conflict.path}\``,
        timestamp: Date.now(),
      };
      setMessages((prev) => [...prev, confirmMessage]);
    } catch (err) {
      const errMsg = typeof err === "string" ? err : "Failed to create copy.";
      setMessages((prev) => [
        ...prev,
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content: `❌ Create copy error: ${errMsg}`,
          timestamp: Date.now(),
        },
      ]);
    }
  };

  const handleCancelConflict = (messageId: string) => {
    setFileConflicts((prev) => {
      const next = { ...prev };
      delete next[messageId];
      return next;
    });
    setResolvedWrites((prev) => ({ ...prev, [messageId]: "denied" }));

    setMessages((prev) => [
      ...prev,
      {
        id: crypto.randomUUID(),
        role: "assistant",
        content: "Write cancelled. The existing file was not modified.",
        timestamp: Date.now(),
      },
    ]);
  };

  const handleDenyWrite = (messageId: string) => {
    setResolvedWrites((prev) => ({ ...prev, [messageId]: "denied" }));
    const cancelMessage: ChatMessage = {
      id: crypto.randomUUID(),
      role: "assistant",
      content: "File write was cancelled.",
      timestamp: Date.now(),
    };
    setMessages((prev) => [...prev, cancelMessage]);
  };

  const handleStartVoice = async () => {
    if (isRecording || isLoading) return;

    setIsRecording(true);
    setErrorMessage(null);
    setAvatarState("listening");

    try {
      const res = await startVoiceInput();

      const userMsgId = crypto.randomUUID();
      const userMessage: ChatMessage = {
        id: userMsgId,
        role: "user",
        content: res.transcription,
        timestamp: Date.now(),
      };

      const assistantMsgId = crypto.randomUUID();
      const assistantMessage: ChatMessage = {
        id: assistantMsgId,
        role: "assistant",
        content: res.response,
        timestamp: Date.now(),
      };

      setMessages((prev) => [...prev, userMessage, assistantMessage]);
      parsePendingWrite(assistantMsgId, res.response);

      const emotion = detectSentiment(res.response);
      setAvatarState(emotion);
      setTimeout(() => {
        setAvatarState("idle");
      }, 3500);
    } catch (err) {
      const errMsg =
        typeof err === "string" ? err : "Failed to record or transcribe voice input.";
      setErrorMessage(errMsg);
      setAvatarState("concerned");
      setTimeout(() => {
        setAvatarState("idle");
      }, 3000);
    } finally {
      setIsRecording(false);
    }
  };

  const handleSend = async () => {
    const text = inputText.trim();
    if (!text || isLoading || isRecording) return;

    // Reset input immediately
    setInputText("");
    setErrorMessage(null);

    const userMessage: ChatMessage = {
      id: crypto.randomUUID(),
      role: "user",
      content: text,
      timestamp: Date.now(),
    };

    setMessages((prev) => [...prev, userMessage]);
    setIsLoading(true);

    // Trigger thinking avatar state while generating
    setAvatarState("thinking");

    try {
      const lower = text.toLowerCase();
      const isScreenQuery =
        lower.includes("screen") ||
        lower.includes("what's on") ||
        lower.includes("whats on") ||
        lower.includes("what do you see");

      let promptToSend = text;
      const isBuiltin = ["play", "coder", "assistant", "personal_friend"].includes(currentMode);

      if (!isBuiltin) {
        try {
          const ext = await modeLoader.loadExtension(currentMode);
          const compName = await getCompanionName().catch(() => "OpenMate");
          const uName = await getUserName().catch(() => "");
          const mems = await getMemories().catch(() => []);
          const systemPrompt = ext.buildSystemPrompt({
            companionName: compName || "OpenMate",
            userName: uName || "",
            memories: mems.map((m) => m.content),
            currentTime: new Date().toLocaleTimeString(),
          });
          promptToSend = `[Custom Persona for ${ext.manifest.name} mode]:\n${systemPrompt}\n\n${text}`;
        } catch (err) {
          console.warn("Failed to apply custom mode system prompt:", err);
        }
      }

      const response = isScreenQuery
        ? await requestScreenContext(text)
        : await sendMessage(promptToSend, isBuiltin ? currentMode : undefined);

      const msgId = crypto.randomUUID();
      const assistantMessage: ChatMessage = {
        id: msgId,
        role: "assistant",
        content: response,
        timestamp: Date.now(),
      };

      setMessages((prev) => [...prev, assistantMessage]);
      parsePendingWrite(msgId, response);

      // Trigger avatar state matching response emotion [Step 2]
      const emotion = detectSentiment(response);
      setAvatarState(emotion);
      setTimeout(() => {
        setAvatarState("idle");
      }, 3500);
    } catch (err) {
      const errMsg =
        typeof err === "string" ? err : "Failed to receive a response from OpenMate.";
      setErrorMessage(errMsg);

      // Trigger concerned avatar state on error
      setAvatarState("concerned");
      setTimeout(() => {
        setAvatarState("idle");
      }, 3000);
    } finally {
      setIsLoading(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  if (!isOpen) return null;

  return (
    <motion.div
      initial={{ opacity: 0, y: 20, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 20, scale: 0.98 }}
      transition={{ duration: 0.2 }}
      className="absolute top-3 left-3 w-[396px] h-[520px] bg-surface-elevated border border-surface-border rounded-2xl shadow-2xl flex flex-col z-40 overflow-hidden"
      style={{ pointerEvents: "auto" }}
    >
      {/* Header with Mode Tabs / Dropdown */}
      <div className="p-3 border-b border-surface-border bg-surface-card flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <span className="text-xs font-semibold uppercase tracking-wider text-neutral-400">
            Companion Mode
          </span>
          <div className="flex items-center gap-1.5">
            {onOpenSettings && (
              <button
                type="button"
                onClick={onOpenSettings}
                title="Open Settings"
                aria-label="Open Settings"
                className="p-1 rounded-lg text-neutral-400 hover:text-white hover:bg-surface-border transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
              >
                <svg viewBox="0 0 24 24" className="w-3.5 h-3.5 fill-none stroke-current" strokeWidth="2">
                  <circle cx="12" cy="12" r="3" />
                  <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
                </svg>
              </button>
            )}
            <button
              type="button"
              onClick={onClose}
              aria-label="Close chat"
              className="text-neutral-400 hover:text-white text-xs px-2 py-0.5 rounded-lg hover:bg-surface-border transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
            >
              Close
            </button>
          </div>
        </div>

        {/* Mode Selector: Tabs if <= 6 modes, Dropdown if > 6 modes */}
        {allModeOptions.length <= 6 ? (
          <div className="flex gap-1 bg-darkBg p-1 rounded-xl border border-surface-border overflow-x-auto">
            {allModeOptions.map((opt) => (
              <button
                key={opt.id}
                type="button"
                onClick={() => handleModeSelect(opt.id)}
                className={`py-1.5 px-2 text-xs font-medium rounded-lg transition-colors flex-1 whitespace-nowrap text-center focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none ${
                  currentMode === opt.id
                    ? "bg-accent text-white shadow-sm"
                    : "text-neutral-400 hover:text-neutral-200 hover:bg-surface-elevated"
                }`}
              >
                {opt.icon ? `${opt.icon} ` : ""}{opt.label}
              </button>
            ))}
          </div>
        ) : (
          <div className="relative">
            <select
              value={currentMode}
              aria-label="Select companion mode"
              onChange={(e) => handleModeSelect(e.target.value as CompanionMode)}
              className="w-full py-1.5 px-3 bg-darkBg border border-surface-border rounded-xl text-xs font-medium text-white focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none cursor-pointer"
            >
              {allModeOptions.map((opt) => (
                <option key={opt.id} value={opt.id} className="bg-darkBg text-white">
                  {opt.icon ? `${opt.icon} ` : ""}{opt.label}
                </option>
              ))}
            </select>
          </div>
        )}

        {/* Mode description */}
        <p className="text-[11px] text-neutral-400 text-center italic">
          {currentModeInfo.description}
        </p>
      </div>

      {/* Message History List */}
      <div className="flex-1 overflow-y-auto p-4 space-y-3">
        {messages.length === 0 && (
          <div className="h-full flex flex-col items-center justify-center text-center p-4 text-neutral-500 text-xs">
            <p>
              Start talking to OpenMate in {currentModeInfo.label} mode.
            </p>
          </div>
        )}

        {messages.map((msg) => {
          const pendingWrite = pendingWrites[msg.id];
          const writeStatus = resolvedWrites[msg.id];
          const isAssistant = msg.role === "assistant";

          return (
            <div key={msg.id} className="space-y-1.5 group">
              <div
                className={`flex items-end gap-1.5 ${
                  msg.role === "user" ? "justify-end" : "justify-start"
                }`}
              >
                <div
                  className={`relative max-w-[85%] rounded-2xl px-3.5 py-2.5 text-xs leading-relaxed break-words shadow-sm ${
                    msg.role === "user"
                      ? "bg-accent text-white rounded-br-none"
                      : "bg-surface-card border border-surface-border text-neutral-200 rounded-bl-none"
                  }`}
                >
                  <div>{parseMarkdown(msg.content)}</div>

                  <div className="flex items-center justify-between gap-2 mt-1 pt-0.5 text-[10px] opacity-60">
                    <span>{formatTimestamp(msg.timestamp)}</span>

                    {/* Copy button on hover for assistant messages */}
                    {isAssistant && (
                      <button
                        type="button"
                        onClick={() => handleCopy(msg.id, msg.content)}
                        title="Copy message"
                        className="opacity-0 group-hover:opacity-100 transition-opacity text-neutral-300 hover:text-white focus:opacity-100"
                      >
                        {copiedId === msg.id ? "✓ Copied" : "Copy"}
                      </button>
                    )}
                  </div>
                </div>
              </div>

              {/* Tool Confirmation UI for Write Operations */}
              {pendingWrite && !writeStatus && (
                <div className="mx-2 p-3 bg-surface-card border border-accent/40 rounded-xl space-y-2 shadow-lg animate-fade-in">
                  <div className="flex items-start gap-2">
                    <span className="text-accent text-sm font-bold">⚠️</span>
                    <div className="flex-1 text-xs">
                      <p className="font-semibold text-white">OpenMate wants to write to:</p>
                      <p className="font-mono text-[11px] text-accent-light break-all mt-0.5">
                        {pendingWrite.path}
                      </p>
                    </div>
                  </div>

                  <div className="flex flex-wrap gap-1.5 pt-1">
                    <button
                      type="button"
                      onClick={() => handleAllowWrite(msg.id, false)}
                      className="px-2.5 py-1 bg-accent hover:bg-accent-hover text-white text-[11px] font-medium rounded-lg transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
                    >
                      Allow this once
                    </button>
                    <button
                      type="button"
                      onClick={() => handleAllowWrite(msg.id, true)}
                      className="px-2.5 py-1 bg-surface-elevated hover:bg-surface-border border border-surface-border text-neutral-200 text-[11px] font-medium rounded-lg transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
                    >
                      Always allow this session
                    </button>
                    <button
                      type="button"
                      onClick={() => handleDenyWrite(msg.id)}
                      className="px-2.5 py-1 bg-status-error/20 hover:bg-status-error/30 text-status-error border border-status-error/30 text-[11px] font-medium rounded-lg transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
                    >
                      Deny
                    </button>
                  </div>
                </div>
              )}

              {/* File Conflict Resolution UI */}
              {fileConflicts[msg.id] && (
                <div className="mx-2 p-3 bg-surface-card border border-amber-500/60 rounded-xl space-y-2.5 shadow-lg animate-fade-in">
                  <div className="flex items-start gap-2">
                    <span className="text-amber-400 text-sm font-bold">⚠️</span>
                    <div className="flex-1 text-xs">
                      <p className="font-semibold text-amber-200">File Already Exists</p>
                      <p className="text-[11px] text-neutral-300 mt-0.5">
                        {fileConflicts[msg.id].errorMessage}
                      </p>
                    </div>
                  </div>

                  <div className="flex flex-wrap gap-1.5 pt-1">
                    <button
                      type="button"
                      onClick={() => handleOverwrite(msg.id)}
                      className="px-2.5 py-1 bg-amber-600 hover:bg-amber-500 text-white text-[11px] font-medium rounded-lg transition-colors focus-visible:ring-2 focus-visible:ring-amber-400 focus-visible:outline-none"
                    >
                      Overwrite
                    </button>
                    <button
                      type="button"
                      onClick={() => handleCreateCopy(msg.id)}
                      className="px-2.5 py-1 bg-accent hover:bg-accent-hover text-white text-[11px] font-medium rounded-lg transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
                    >
                      Create Copy
                    </button>
                    <button
                      type="button"
                      onClick={() => handleCancelConflict(msg.id)}
                      className="px-2.5 py-1 bg-surface-elevated hover:bg-surface-border border border-surface-border text-neutral-300 text-[11px] font-medium rounded-lg transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              )}
            </div>
          );
        })}

        {/* Typing indicator (3 bouncing dots) while waiting for Gemini */}
        {isLoading && (
          <div className="flex justify-start animate-fade-in">
            <div className="bg-surface-card border border-surface-border rounded-2xl rounded-bl-none px-3.5 py-2.5 text-xs text-neutral-400 flex items-center gap-1.5">
              <span className="w-1.5 h-1.5 rounded-full bg-accent animate-bounce" />
              <span className="w-1.5 h-1.5 rounded-full bg-accent animate-bounce [animation-delay:0.2s]" />
              <span className="w-1.5 h-1.5 rounded-full bg-accent animate-bounce [animation-delay:0.4s]" />
              <span className="ml-1 text-[11px]">Thinking...</span>
            </div>
          </div>
        )}

        {isRecording && (
          <div className="flex justify-start animate-fade-in">
            <div className="bg-status-error/10 border border-status-error/30 rounded-2xl rounded-bl-none px-3.5 py-2.5 text-xs text-status-error flex items-center gap-2">
              <span className="w-2 h-2 rounded-full bg-status-error animate-ping" />
              <span className="font-medium">Listening to your microphone (5s)...</span>
            </div>
          </div>
        )}

        {errorMessage && (
          <div className="bg-status-error/10 border border-status-error/30 text-status-error rounded-xl p-2.5 text-xs">
            <p>{errorMessage}</p>
          </div>
        )}

        <div ref={messagesEndRef} />
      </div>

      {/* Input Area */}
      <div className="p-3 border-t border-surface-border bg-surface-card flex items-end gap-2">
        <textarea
          ref={textareaRef}
          rows={2}
          value={inputText}
          onChange={(e) => setInputText(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Ask anything (Enter to send, Shift+Enter for newline)..."
          className="flex-1 bg-darkBg border border-surface-border rounded-xl px-3 py-2 text-xs text-white placeholder-neutral-500 resize-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
        />
        {/* Microphone Button */}
        <button
          type="button"
          onClick={handleStartVoice}
          disabled={isLoading || isRecording}
          title="Voice input (5 seconds)"
          aria-label="Start voice input"
          className={`h-10 w-10 flex items-center justify-center rounded-xl border transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none ${
            isRecording
              ? "bg-status-error/20 border-status-error text-status-error animate-pulse"
              : "bg-surface-elevated hover:bg-surface-border border-surface-border text-neutral-300 hover:text-white"
          }`}
        >
          {isRecording ? (
            <span className="w-2.5 h-2.5 rounded-full bg-status-error" />
          ) : (
            <svg viewBox="0 0 24 24" className="w-4 h-4 fill-none stroke-current" strokeWidth="2">
              <path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z" />
              <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
              <line x1="12" y1="19" x2="12" y2="23" />
              <line x1="8" y1="23" x2="16" y2="23" />
            </svg>
          )}
        </button>
        {/* Send Button */}
        <button
          type="button"
          onClick={handleSend}
          disabled={isLoading || isRecording || !inputText.trim()}
          className="h-10 px-3.5 bg-accent hover:bg-accent-hover disabled:opacity-40 disabled:cursor-not-allowed text-white text-xs font-medium rounded-xl flex items-center justify-center transition-colors focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
        >
          {isLoading ? "..." : "Send"}
        </button>
      </div>
    </motion.div>
  );
}
