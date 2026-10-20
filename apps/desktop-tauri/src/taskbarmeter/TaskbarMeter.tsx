import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useLocale } from "../hooks/useLocale";
import { useProviders } from "../hooks/useProviders";
import { getSettingsSnapshot, refreshProvidersIfStale } from "../lib/tauri";
import { describeProviderState } from "../lib/providerState";
import { DetailedProviderTile, ProviderPill } from "../floatbar/FloatBar";
import { detailedLanes } from "../floatbar/detailedMeters";
import type { BootstrapState, ProviderUsageSnapshot } from "../types/bridge";
import "../floatbar/FloatBar.css";
import "./TaskbarMeter.css";

/**
 * Taskbar meter: one tile per enabled provider (its current account) with the
 * session and weekly meters, docked on the Windows taskbar by the native side.
 * The whole strip is a drag handle that only slides along the taskbar.
 */
export default function TaskbarMeter({ state }: { state: BootstrapState }) {
  const { t } = useLocale();
  const { providers } = useProviders({ refreshOnMount: false });
  const [settings, setSettings] = useState(state.settings);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    document.body.classList.add("floatbar-window", "taskbar-meter-window");
    return () => document.body.classList.remove("floatbar-window", "taskbar-meter-window");
  }, []);

  useEffect(() => {
    const unlisten = listen("settings-changed", () => {
      void getSettingsSnapshot().then(setSettings).catch(() => {});
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(id);
  }, []);

  // Keep usage fresh without opening any surface (same cadence as the floatbar).
  useEffect(() => {
    const baseMs = Math.max(60_000, settings.refreshIntervalSecs * 1000);
    const intervalMs = settings.lowPowerMode ? Math.max(baseMs, 30 * 60 * 1000) : baseMs;
    const tick = () => void refreshProvidersIfStale().catch(() => {});
    tick();
    const id = window.setInterval(tick, intervalMs);
    return () => window.clearInterval(id);
  }, [settings.refreshIntervalSecs, settings.lowPowerMode]);

  const visible = useMemo(() => {
    const order = settings.enabledProviders;
    const rank = new Map(order.map((id, index) => [id, index]));
    return providers
      .filter((p) => rank.has(p.providerId))
      .sort((a, b) => (rank.get(a.providerId) ?? 0) - (rank.get(b.providerId) ?? 0));
  }, [providers, settings.enabledProviders]);

  // Fit the native window to the content (physical px).
  const rootRef = useRef<HTMLDivElement | null>(null);
  const lastSize = useRef<string>("");
  const fit = useCallback(() => {
    const el = rootRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const dpr = window.devicePixelRatio > 0 ? window.devicePixelRatio : 1;
    const w = Math.ceil((rect.width + 4) * dpr);
    const h = Math.ceil((rect.height + 4) * dpr);
    const key = `${w}x${h}`;
    if (key === lastSize.current) return;
    lastSize.current = key;
    void invoke("taskbar_meter_resize", { width: w, height: h }).catch(() => {});
  }, []);
  useEffect(() => {
    fit();
  }, [fit, visible.length, now]);
  useEffect(() => {
    const el = rootRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(fit);
    observer.observe(el);
    return () => observer.disconnect();
  }, [fit]);

  // Slide along the taskbar. The native side tracks the cursor until the
  // physical button is released, so a lost pointerup can't leave it stuck
  // following the mouse. Locked meters ignore the press.
  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || settings.taskbarMeterLocked) return;
    void invoke("taskbar_meter_drag_start").catch(() => {});
  };

  const renderProvider = (p: ProviderUsageSnapshot) =>
    !describeProviderState(p.errorState).isProblem && detailedLanes(p).length > 0 ? (
      <DetailedProviderTile
        key={p.providerId}
        provider={p}
        thresholds={settings}
        showAsUsed={settings.showAsUsed}
        scale={1}
        now={now}
        usedSuffix={t("PanelUsedSuffix")}
        remainingSuffix={t("FloatBarRemainingSuffix")}
      />
    ) : (
      <ProviderPill
        key={p.providerId}
        provider={p}
        thresholds={settings}
        showAsUsed={settings.showAsUsed}
        scale={1}
        showResetInline
        resetRelative={settings.resetTimeRelative}
        usedSuffix={t("PanelUsedSuffix")}
        remainingSuffix={t("FloatBarRemainingSuffix")}
        stateLabel={t(describeProviderState(p.errorState).labelKey)}
      />
    );

  return (
    <div
      ref={rootRef}
      className={`floatbar floatbar--horizontal floatbar--floating floatbar--detailed taskbar-meter${
        settings.taskbarMeterLocked ? " taskbar-meter--locked" : ""
      }`}
      onPointerDown={onPointerDown}
      onContextMenu={(event) => {
        event.preventDefault();
        void invoke("taskbar_meter_context_menu").catch(() => {});
      }}
      aria-label={t("TaskbarMeterSectionTitle")}
    >
      {visible.length === 0 ? (
        <div className="floatbar__empty">{t("FloatBarNoProviders")}</div>
      ) : (
        visible.map(renderProvider)
      )}
    </div>
  );
}
