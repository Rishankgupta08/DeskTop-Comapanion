/**
 * OpenMate Memory Management UI
 *
 * Displays saved user memories with deletion and clear-all capabilities.
 * [SRS FR-037, FR-038, FR-039]
 */

import { useState } from "react";
import type { MemoryEntry } from "../../types";

interface MemoryPanelProps {
  memories: MemoryEntry[];
  onDelete: (id: string) => Promise<void> | void;
  onClearAll: () => Promise<void> | void;
}

export default function MemoryPanel({
  memories,
  onDelete,
  onClearAll,
}: MemoryPanelProps) {
  const [confirmClear, setConfirmClear] = useState(false);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [isClearing, setIsClearing] = useState(false);

  const handleDelete = async (id: string) => {
    setDeletingId(id);
    try {
      await onDelete(id);
    } finally {
      setDeletingId(null);
    }
  };

  const handleConfirmClear = async () => {
    setIsClearing(true);
    try {
      await onClearAll();
      setConfirmClear(false);
    } finally {
      setIsClearing(false);
    }
  };

  return (
    <div className="space-y-3">
      {memories.length === 0 ? (
        <p className="text-xs text-neutral-500 italic py-2">
          No saved memories yet. OpenMate remembers useful details as you interact.
        </p>
      ) : (
        <div className="space-y-2 max-h-48 overflow-y-auto pr-1">
          {memories.map((mem) => {
            const preview =
              mem.content.length > 80
                ? `${mem.content.slice(0, 80)}...`
                : mem.content;

            return (
              <div
                key={mem.id}
                className="flex items-center justify-between gap-3 bg-surface-card border border-surface-border rounded-xl p-2.5 text-xs text-neutral-300"
              >
                <div className="flex-1 min-w-0">
                  <p className="truncate text-white">{preview}</p>
                  {mem.tags && mem.tags.length > 0 && (
                    <div className="flex gap-1 mt-1">
                      {mem.tags.map((t, idx) => (
                        <span
                          key={idx}
                          className="px-1.5 py-0.5 bg-surface-elevated text-neutral-400 text-[10px] rounded"
                        >
                          {t}
                        </span>
                      ))}
                    </div>
                  )}
                </div>

                <button
                  type="button"
                  onClick={() => handleDelete(mem.id)}
                  disabled={deletingId === mem.id}
                  className="px-2 py-1 text-xs text-status-error hover:bg-status-error/10 border border-transparent hover:border-status-error/20 rounded-lg transition-colors focus-visible:ring-2 focus-visible:ring-status-error focus-visible:outline-none"
                >
                  {deletingId === mem.id ? "..." : "Delete"}
                </button>
              </div>
            );
          })}
        </div>
      )}

      {memories.length > 0 && (
        <div className="pt-2 border-t border-surface-border">
          {!confirmClear ? (
            <button
              type="button"
              onClick={() => setConfirmClear(true)}
              className="text-xs text-status-error hover:underline focus-visible:ring-2 focus-visible:ring-status-error focus-visible:outline-none rounded"
            >
              Clear all memories
            </button>
          ) : (
            <div className="flex items-center gap-3 bg-status-error/10 border border-status-error/20 p-2.5 rounded-xl">
              <span className="text-xs text-status-error font-medium">
                Are you sure?
              </span>
              <button
                type="button"
                onClick={handleConfirmClear}
                disabled={isClearing}
                className="px-2.5 py-1 bg-status-error text-white text-xs font-medium rounded-lg hover:bg-red-600 focus-visible:ring-2 focus-visible:ring-status-error focus-visible:outline-none"
              >
                {isClearing ? "Clearing..." : "Yes, Clear All"}
              </button>
              <button
                type="button"
                onClick={() => setConfirmClear(false)}
                className="px-2.5 py-1 text-xs text-neutral-300 hover:text-white rounded-lg hover:bg-surface-card focus-visible:ring-2 focus-visible:ring-accent focus-visible:outline-none"
              >
                Cancel
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
