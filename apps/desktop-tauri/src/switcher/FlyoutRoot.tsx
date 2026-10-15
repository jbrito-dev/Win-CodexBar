import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import TrayPanel from "../surfaces/TrayPanel";
import AccountSwitcher from "./AccountSwitcher";
import type { BootstrapState } from "../types/bridge";

export type FlyoutMode = "switcher" | "dashboard";

/**
 * Content of the flyout window: the dark account switcher (tray left-click)
 * or the full dashboard (Pop Out Dashboard, shortcut). The native side owns
 * the mode and announces swaps with `flyout-mode-changed`.
 */
export default function FlyoutRoot({ state }: { state: BootstrapState }) {
  const [mode, setMode] = useState<FlyoutMode | null>(null);

  useEffect(() => {
    let cancelled = false;
    void invoke<string>("get_flyout_mode")
      .then((value) => {
        if (!cancelled) setMode(value === "switcher" ? "switcher" : "dashboard");
      })
      .catch(() => {
        if (!cancelled) setMode("dashboard");
      });
    const unlisten = listen<string>("flyout-mode-changed", (event) => {
      setMode(event.payload === "switcher" ? "switcher" : "dashboard");
    });
    return () => {
      cancelled = true;
      void unlisten.then((fn) => fn());
    };
  }, []);

  if (mode === null) return null;
  return mode === "switcher" ? <AccountSwitcher state={state} /> : <TrayPanel state={state} />;
}
