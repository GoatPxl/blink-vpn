"use client";

import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";
import { PixelIcon } from "@/components/ui/pixel-icon";

type TauriWindow = Window & { __TAURI_INTERNALS__?: unknown };

/** The window is ours only inside the desktop shell; a plain browser has no window to drive. */
function inDesktopShell(): boolean {
  return typeof window !== "undefined" && !!(window as TauriWindow).__TAURI_INTERNALS__;
}

/**
 * The window's title bar. Native decorations are off (`tauri.conf.json`), so this draws it.
 *
 * Close asks the window to close rather than quitting: the Rust `CloseRequested` handler decides
 * whether that hides to the tray, exactly as it did for the native button.
 */
export function Titlebar() {
  const [maximized, setMaximized] = useState(false);
  const [desktop, setDesktop] = useState(false);

  useEffect(() => {
    if (!inDesktopShell()) return;
    const appWindow = getCurrentWindow();
    let disposed = false;
    let unlisten: (() => void) | null = null;

    const sync = () => {
      void appWindow.isMaximized().then((value) => {
        if (!disposed) setMaximized(value);
      });
    };

    // Deferred so the effect body does not set state synchronously.
    const ready = window.setTimeout(() => setDesktop(true), 0);
    sync();
    void appWindow.onResized(sync).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });

    return () => {
      disposed = true;
      window.clearTimeout(ready);
      unlisten?.();
    };
  }, []);

  const control = "flex h-9 w-[46px] items-center justify-center text-stone transition-colors hover:bg-panel-2 hover:text-bone";

  return (
    <div
      data-tauri-drag-region
      className="flex h-[var(--titlebar-h)] shrink-0 select-none items-center justify-between border-b-2 border-line bg-ink"
    >
      <span data-tauri-drag-region className="pl-4 font-display text-[12px] uppercase tracking-[0.06em] text-stone">
        Blink VPN
      </span>
      {desktop ? (
        <div className="flex">
          <button type="button" aria-label="Minimize" className={control} onClick={() => void getCurrentWindow().minimize()}>
            <PixelIcon name="minimize" className="h-3 w-3" />
          </button>
          <button
            type="button"
            aria-label={maximized ? "Restore" : "Maximize"}
            className={control}
            onClick={() => void getCurrentWindow().toggleMaximize()}
          >
            <PixelIcon name={maximized ? "restore" : "maximize"} className="h-3 w-3" />
          </button>
          <button
            type="button"
            aria-label="Close"
            className={`${control} hover:!bg-alert hover:!text-ink`}
            onClick={() => void getCurrentWindow().close()}
          >
            <PixelIcon name="close" className="h-3 w-3" />
          </button>
        </div>
      ) : null}
    </div>
  );
}
