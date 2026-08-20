import {
  getCurrentWindow,
  LogicalPosition,
  LogicalSize,
  currentMonitor,
} from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";

export type WindowState = "CLOSED" | "CHAT" | "SETTINGS" | "BUBBLE";

export const WINDOW_SIZES: Record<WindowState, { width: number; height: number }> = {
  CLOSED: { width: 120, height: 120 },
  CHAT: { width: 420, height: 640 },
  SETTINGS: { width: 680, height: 750 },
  BUBBLE: { width: 320, height: 200 },
};

let currentLayoutState: WindowState = "CLOSED";
let currentLayoutWidth: number = 120;
let currentLayoutHeight: number = 120;

/**
 * Initializes the native companion window size and position at launch.
 */
export async function initializeWindowLayout(): Promise<void> {
  console.log("[WindowLayout] Initializing window layout to CLOSED (120x120)");
  try {
    await invoke("set_window_layout_state", {
      layoutState: "CLOSED",
      customWidth: 120,
      customHeight: 120,
    });
    currentLayoutState = "CLOSED";
    currentLayoutWidth = 120;
    currentLayoutHeight = 120;
  } catch (err) {
    console.warn("[WindowLayout] Rust IPC initialize failed, trying JS API fallback:", err);
    try {
      const appWindow = getCurrentWindow();
      const size = WINDOW_SIZES.CLOSED;
      await appWindow.setSize(new LogicalSize(size.width, size.height));

      const factor = await appWindow.scaleFactor();
      const physPos = await appWindow.outerPosition();
      const currentX = physPos.x / factor;
      const currentY = physPos.y / factor;

      if (currentX <= 50 && currentY <= 50) {
        const monitor = await currentMonitor();
        if (monitor) {
          const scaleFactor = monitor.scaleFactor || 1;
          const screenW = monitor.size.width / scaleFactor;
          const screenH = monitor.size.height / scaleFactor;

          const initX = Math.max(0, screenW - size.width - 24);
          const initY = Math.max(0, screenH - size.height - 24);
          await appWindow.setPosition(new LogicalPosition(initX, initY));
        }
      }
      currentLayoutState = "CLOSED";
      currentLayoutWidth = 120;
      currentLayoutHeight = 120;
    } catch (fallbackErr) {
      console.error("[WindowLayout] JS API fallback initialize failed:", fallbackErr);
    }
  }
}

/**
 * Dynamically resizes and repositions the native Tauri window
 * so the companion avatar remains visually anchored at the same desktop location.
 */
export async function setWindowLayoutState(
  newState: WindowState,
  customWidth?: number,
  customHeight?: number
): Promise<void> {
  const targetW = customWidth || WINDOW_SIZES[newState].width;
  const targetH = customHeight || WINDOW_SIZES[newState].height;

  if (
    currentLayoutState === newState &&
    Math.abs(currentLayoutWidth - targetW) < 2 &&
    Math.abs(currentLayoutHeight - targetH) < 2
  ) {
    return;
  }

  console.log(
    `[WindowLayout] Transitioning window state: ${currentLayoutState} (${currentLayoutWidth}x${currentLayoutHeight}) -> ${newState} (${targetW}x${targetH})`
  );

  try {
    await invoke("set_window_layout_state", {
      layoutState: newState,
      customWidth: targetW,
      customHeight: targetH,
    });
    console.log(`[WindowLayout] Transition to ${newState} (${targetW}x${targetH}) succeeded via Rust IPC`);
    currentLayoutState = newState;
    currentLayoutWidth = targetW;
    currentLayoutHeight = targetH;
  } catch (err) {
    console.warn(`[WindowLayout] Rust IPC set_window_layout_state failed for ${newState}:`, err);
    try {
      const appWindow = getCurrentWindow();
      const oldW = currentLayoutWidth;
      const oldH = currentLayoutHeight;
      const newW = targetW;
      const newH = targetH;

      const factor = await appWindow.scaleFactor();
      const physPos = await appWindow.outerPosition();
      const currentX = physPos.x / factor;
      const currentY = physPos.y / factor;

      const monitor = await currentMonitor();
      let minX = 0;
      let minY = 0;
      let maxX = 1920;
      let maxY = 1080;

      if (monitor) {
        const monScale = monitor.scaleFactor || factor || 1;
        minX = monitor.position.x / monScale;
        minY = monitor.position.y / monScale;
        maxX = minX + monitor.size.width / monScale;
        maxY = minY + monitor.size.height / monScale;
      }

      const anchorX = currentX + oldW;
      const anchorY = currentY + oldH;

      let newX = anchorX - newW;
      let newY = anchorY - newH;

      // Clamp within monitor usable area
      if (newX < minX) newX = Math.max(minX, currentX);
      if (newX + newW > maxX) newX = Math.max(minX, maxX - newW);
      if (newY < minY) newY = Math.max(minY, currentY);
      if (newY + newH > maxY) newY = Math.max(minY, maxY - newH);

      console.log(
        `[WindowLayout JS Fallback] Current=(${currentX},${currentY}), Anchor=(${anchorX},${anchorY}), Target=(${newX},${newY}, ${newW}x${newH})`
      );

      if (newW > oldW || newH > oldH) {
        await appWindow.setPosition(new LogicalPosition(newX, newY));
        await appWindow.setSize(new LogicalSize(newW, newH));
      } else {
        await appWindow.setSize(new LogicalSize(newW, newH));
        await appWindow.setPosition(new LogicalPosition(newX, newY));
      }

      currentLayoutState = newState;
      currentLayoutWidth = newW;
      currentLayoutHeight = newH;
      console.log(`[WindowLayout JS Fallback] Transition to ${newState} completed`);
    } catch (fallbackErr) {
      console.error(`[WindowLayout JS Fallback] Failed to transition to ${newState}:`, fallbackErr);
    }
  }
}

/**
 * Call on mouse down on avatar to trigger native OS window dragging.
 */
export async function startNativeDrag(): Promise<void> {
  try {
    const appWindow = getCurrentWindow();
    await appWindow.startDragging();
  } catch (err) {
    try {
      await invoke("start_window_drag");
    } catch (ipcErr) {
      console.warn("[WindowLayout] Failed to start native window drag:", err, ipcErr);
    }
  }
}
