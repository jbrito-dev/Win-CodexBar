import { Suspense, lazy, useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  checkForUpdates,
  downloadUpdate,
  getBootstrapState,
  getSettingsSnapshot,
} from "./lib/tauri";
import { useSurfaceSnapshot } from "./hooks/useSurfaceSnapshot";
import { useTheme } from "./hooks/useTheme";
import { useLocale } from "./hooks/useLocale";
import TrayPanel from "./surfaces/TrayPanel";
import FlyoutRoot from "./switcher/FlyoutRoot";
import { FLOATBAR_WINDOW_LABEL } from "./floatbar/api";
import { LocaleProvider } from "./i18n/LocaleProvider";
import type { BootstrapState, ThemePreference } from "./types/bridge";
import type { SurfaceSnapshot } from "./hooks/useSurfaceSnapshot";
import { useDeepSeekPricingStatus } from "./hooks/useDeepSeekPricingStatus";
import { CurrencyProvider } from "./hooks/CurrencyProvider";

const Settings = lazy(() => import("./surfaces/Settings"));
const FloatBar = lazy(() => import("./floatbar/FloatBar"));
const TaskbarMeter = lazy(() => import("./taskbarmeter/TaskbarMeter"));

function SurfaceFallback() {
  return null;
}

/** True when running inside the detached Settings window. */
function isSettingsWindow(): boolean {
  return getCurrentWebviewWindow().label === "settings";
}

/** True when running inside the detached FloatBar window. */
function isFloatBarWindow(): boolean {
  return getCurrentWebviewWindow().label === FLOATBAR_WINDOW_LABEL;
}

/** True when running inside the taskbar-docked meter window. */
function isTaskbarMeterWindow(): boolean {
  return getCurrentWebviewWindow().label === "taskbarmeter";
}

/** True when running inside the detached flyout ("Pop Out Dashboard") window. */
function isFlyoutWindow(): boolean {
  return getCurrentWebviewWindow().label === "flyout";
}

/** Parse the initial Settings tab from the URL query string. */
function initialSettingsTab(): string {
  const params = new URLSearchParams(window.location.search);
  return params.get("tab") || "general";
}

export default function App() {
  return (
    <LocaleProvider>
      <CurrencyProvider>
        <AppInner />
      </CurrencyProvider>
    </LocaleProvider>
  );
}

function AppInner() {
  const { t } = useLocale();
  const surface = useSurfaceSnapshot();
  const [state, setState] = useState<BootstrapState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [themePreference, setThemePreference] = useState<ThemePreference>("dark");

  useTheme(themePreference);
  useDeepSeekPricingStatus();

  const reloadBootstrapState = useCallback(
    () => getBootstrapState(),
    [],
  );

  useEffect(() => {
    let cancelled = false;

    reloadBootstrapState()
      .then((bootstrap) => {
        if (cancelled) {
          return;
        }
        setState(bootstrap);
        setThemePreference(bootstrap.settings.theme);
        setError(null);
      })
      .catch((cause: unknown) => {
        if (!cancelled) {
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      });

    // Fire-and-forget update checks after the first paint so startup/tray open
    // is not competing with network work.
    const updateTimer = window.setTimeout(() => {
      Promise.all([checkForUpdates(), getSettingsSnapshot()])
        .then(([update, settings]) => {
          if (settings.autoDownloadUpdates && update.canDownload) {
            void downloadUpdate().catch(() => {});
          }
        })
        .catch(() => {});
    }, 2_000);

    // Global shortcuts (the persisted one and ad-hoc capture registrations)
    // are handled natively: shortcut_bridge::plugin's handler toggles the
    // tray-panel flyout for every registered shortcut, so no frontend
    // listener opens a second window here.

    const unlistenSettingsChangePromise = isSettingsWindow()
      ? listen<string>("settings-change-tab", () => {
          void reloadBootstrapState()
            .then((bootstrap) => {
              setState(bootstrap);
              setThemePreference(bootstrap.settings.theme);
              setError(null);
            })
            .catch(() => {});
        })
      : Promise.resolve(null);

    // Keep the theme in sync when mutations happen inside other surfaces
    // (e.g., Settings → Appearance). `useSettings` dispatches this event
    // after every successful `updateSettings` call.
    const onSettingsUpdated = (evt: Event) => {
      const detail = (evt as CustomEvent<BootstrapState["settings"]>).detail;
      if (detail) {
        setThemePreference(detail.theme);
      } else {
        getSettingsSnapshot()
          .then((fresh) => setThemePreference(fresh.theme))
          .catch(() => {});
      }
    };
    window.addEventListener("codexbar:settings-updated", onSettingsUpdated);

    return () => {
      cancelled = true;
      void unlistenSettingsChangePromise
        .then((unlisten) => unlisten?.())
        .catch(() => {});
      window.clearTimeout(updateTimer);
      window.removeEventListener("codexbar:settings-updated", onSettingsUpdated);
    };
  }, [reloadBootstrapState]);

  if (error) {
    return (
      <main className="shell">
        <section className="panel error">
          <h2>{t("BootstrapFailed")}</h2>
          <p>{error}</p>
        </section>
      </main>
    );
  }

  if (!state) {
    return (
      <main className="shell">
        <section className="panel">
          <h2>{t("LoadingShellContract")}</h2>
          <p>{t("LoadingShellContractHint")}</p>
        </section>
      </main>
    );
  }

  // Detached settings window — render Settings directly, skip SurfaceRouter.
  if (isSettingsWindow()) {
    return <DetachedSettingsApp state={state} />;
  }

  // Detached floating-bar window — render the FloatBar surface directly.
  if (isFloatBarWindow()) {
    return (
      <Suspense fallback={<SurfaceFallback />}>
        <FloatBar state={state} />
      </Suspense>
    );
  }

  // Detached flyout ("Pop Out Dashboard") window — render TrayPanel directly.
  // This is the only dashboard layout; the legacy PopOut layout is retired.
  // TrayPanel is statically imported (not lazy), so no Suspense boundary is
  // needed here, unlike the other detached-window branches above.
  if (isFlyoutWindow()) {
    return <FlyoutRoot state={state} />;
  }

  if (isTaskbarMeterWindow()) {
    return (
      <Suspense fallback={<SurfaceFallback />}>
        <TaskbarMeter state={state} />
      </Suspense>
    );
  }

  return <SurfaceRouter surface={surface} state={state} />;
}

function SurfaceRouter({
  surface,
  state,
}: {
  surface: SurfaceSnapshot;
  state: BootstrapState;
}) {
  switch (surface.mode) {
    case "hidden":
      return null;
    case "settings":
      return (
        <Suspense fallback={<SurfaceFallback />}>
          <SettingsLayout state={state} />
        </Suspense>
      );
    default:
      return <TrayPanel state={state} />;
  }
}

function SettingsLayout({ state }: { state: BootstrapState }) {
  return (
    <main className="settings-surface settings-surface--full">
      <Settings state={state} />
    </main>
  );
}

function DetachedSettingsApp({ state }: { state: BootstrapState }) {
  const [request, setRequest] = useState(() => ({ tab: initialSettingsTab(), id: 0 }));

  useEffect(() => {
    // Listen for tab-change events from Rust (when the window is re-focused
    // with a different tab request). Each event gets a new id so repeating
    // the same `providers:<id>` deep link selects that provider again.
    const unlisten = listen<string>("settings-change-tab", (event) => {
      setRequest((current) => ({ tab: event.payload, id: current.id + 1 }));
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  return (
    <Suspense fallback={<SurfaceFallback />}>
      <main className="settings-surface settings-surface--full">
        <Settings state={state} initialTab={request.tab} tabRequestId={request.id} />
      </main>
    </Suspense>
  );
}
