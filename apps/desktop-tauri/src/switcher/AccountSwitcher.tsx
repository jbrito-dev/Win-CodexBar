import { useCallback, useEffect, useMemo, useState, type CSSProperties } from "react";
import { listen } from "@tauri-apps/api/event";
import { useLocale } from "../hooks/useLocale";
import { useProviders } from "../hooks/useProviders";
import { useTrayPanelLayout } from "../hooks/useTrayPanelLayout";
import {
  claudeAccountSwitch,
  claudeAccountsList,
  codexAccountRestartDesktop,
  codexAccountSwitch,
  getCodexAccountsState,
  getSettingsSnapshot,
  openFlyoutWindow,
  openSettingsWindow,
  quitApp,
  refreshProviders,
  refreshProvidersIfStale,
} from "../lib/tauri";
import { ProviderIcon } from "../components/providers/ProviderIcon";
import { formatResetDateTime, resetCountdown } from "../floatbar/detailedMeters";
import type { BootstrapState, ClaudeAccount, CodexAccountsStateBridge } from "../types/bridge";
import { claudeRows, codexRows, type SwitcherLane, type SwitcherProvider, type SwitcherRow } from "./accountRows";
import "./AccountSwitcher.css";

function intlLocale(locale: string): string | undefined {
  return /^[a-z]{2}-[A-Z]{2}$/.test(locale) ? locale : undefined;
}

/**
 * Tray left-click surface: every saved Claude Code and Codex account with its
 * session and weekly usage. Clicking an inactive account switches that
 * provider (each provider switches independently). Everything else lives on
 * the tray icon's right-click menu and in the dashboard.
 */
export default function AccountSwitcher({ state }: { state: BootstrapState }) {
  const { t } = useLocale();
  const { providers, isRefreshing, hasLoadedCache } = useProviders({ refreshOnMount: false });
  const [settings, setSettings] = useState(state.settings);
  const [claudeAccounts, setClaudeAccounts] = useState<ClaudeAccount[]>([]);
  const [codexState, setCodexState] = useState<CodexAccountsStateBridge | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [switching, setSwitching] = useState<string | null>(null);
  const [message, setMessage] = useState<{ kind: "ok" | "error"; text: string } | null>(null);
  const [now, setNow] = useState(() => Date.now());

  const load = useCallback(async () => {
    const [claude, codex] = await Promise.allSettled([claudeAccountsList(), getCodexAccountsState()]);
    if (claude.status === "fulfilled") setClaudeAccounts(claude.value);
    if (codex.status === "fulfilled") setCodexState(codex.value);
    setLoaded(true);
  }, []);

  useEffect(() => {
    void load();
    void refreshProvidersIfStale().catch(() => {});
    const unlisteners = [
      listen("claude-accounts-updated", () => void load()),
      listen("codex-accounts-updated", () => void load()),
      listen("settings-changed", () => {
        void load();
        void getSettingsSnapshot().then(setSettings).catch(() => {});
      }),
      // Re-read whenever the tray swaps this window back to the switcher.
      listen("flyout-mode-changed", (event) => {
        if (event.payload === "switcher") {
          setMessage(null);
          void load();
          void refreshProvidersIfStale().catch(() => {});
        }
      }),
    ];
    const tick = window.setInterval(() => {
      setNow(Date.now());
      void load();
    }, 60_000);
    return () => {
      window.clearInterval(tick);
      for (const unlisten of unlisteners) void unlisten.then((fn) => fn());
    };
  }, [load]);

  const enabled = useMemo(() => new Set(settings.enabledProviders), [settings.enabledProviders]);
  const snapshotFor = (id: string) => providers.find((p) => p.providerId === id);
  const sections: Array<{ provider: SwitcherProvider; title: string; rows: SwitcherRow[] }> = [];
  if (enabled.has("claude") || claudeAccounts.length > 0) {
    sections.push({ provider: "claude", title: "Claude", rows: claudeRows(claudeAccounts, snapshotFor("claude")) });
  }
  if (enabled.has("codex") || (codexState?.accounts.length ?? 0) > 0) {
    sections.push({ provider: "codex", title: "Codex", rows: codexRows(codexState, snapshotFor("codex")) });
  }

  const layoutKey = [
    loaded,
    message?.text ?? "",
    switching ?? "",
    ...sections.map((s) => `${s.provider}:${s.rows.map((r) => `${r.title}${r.active}${r.lanes.length}${r.note ?? ""}`).join(",")}`),
  ].join("|");
  const { layoutReady, requestLayout } = useTrayPanelLayout({
    canMeasure: loaded || hasLoadedCache,
    denseOverview: false,
    detailMode: false,
    layoutKey,
  });
  useEffect(() => {
    requestLayout();
  }, [requestLayout, layoutKey]);

  const switchTo = async (row: SwitcherRow) => {
    if (!row.id || switching) return;
    setSwitching(`${row.provider}:${row.id}`);
    setMessage(null);
    try {
      if (row.provider === "claude") {
        await claudeAccountSwitch(row.id);
        setMessage({ kind: "ok", text: t("ClaudeAccountsSwitched") });
      } else {
        const result = await codexAccountSwitch(row.id);
        // Close and relaunch Codex Desktop on the new account. A stopped
        // Desktop only gets its session state swapped; a missing Desktop
        // install is not an error for the switch itself.
        await codexAccountRestartDesktop(result.switchId, false).catch(() => {});
      }
      await load();
    } catch (error: unknown) {
      setMessage({ kind: "error", text: error instanceof Error ? error.message : String(error) });
    } finally {
      setSwitching(null);
    }
  };

  const showAsUsed = settings.showAsUsed;
  const locale = intlLocale(t("IntlLocale"));

  return (
    <div className={`tray-panel-reveal switcher-reveal${layoutReady ? " tray-panel-reveal--ready" : ""}`} data-theme="dark">
      <div className="menu-surface menu-surface--tray switcher">
        <header className="switcher__header">
          <span className="switcher__brand">{t("AppName")}</span>
          <button
            type="button"
            className={`switcher__icon-button${isRefreshing ? " switcher__icon-button--spinning" : ""}`}
            title={t("ActionRefresh")}
            aria-label={t("ActionRefresh")}
            onClick={() => void refreshProviders().catch(() => {})}
          >
            <svg viewBox="0 0 16 16" width="14" height="14" fill="none" aria-hidden="true">
              <path d="M12.9 7.1a5 5 0 1 0-1.2 3.9" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
              <path d="M12.9 3.8v3.3H9.6" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </button>
        </header>
        <div className="menu-surface__body switcher__body">
          {sections.map((section) => (
            <section key={section.provider} className="switcher__section">
              <div className="switcher__section-title">
                <ProviderIcon providerId={section.provider} size={14} />
                <span>{section.title}</span>
              </div>
              {section.rows.length === 0 ? (
                <div className="switcher__empty">
                  {t(section.provider === "claude" ? "ClaudeAccountsEmpty" : "CodexAccountsEmpty")}
                </div>
              ) : (
                section.rows.map((row) => (
                  <AccountCard
                    key={`${row.provider}:${row.id ?? "active"}:${row.title}`}
                    row={row}
                    busy={switching === `${row.provider}:${row.id}`}
                    disabled={switching !== null}
                    showAsUsed={showAsUsed}
                    now={now}
                    locale={locale}
                    t={t}
                    onSwitch={() => void switchTo(row)}
                  />
                ))
              )}
            </section>
          ))}
          {message && (
            <div className={`switcher__message switcher__message--${message.kind}`} role="status">
              {message.text}
            </div>
          )}
        </div>
        <footer className="menu-surface__footer switcher__footer">
          <button type="button" className="switcher__link" onClick={() => void openFlyoutWindow().catch(() => {})}>
            {t("TrayPopOutDashboard")}
          </button>
          <span className="switcher__footer-actions">
            <button type="button" className="switcher__link" onClick={() => void openSettingsWindow("general").catch(() => {})}>
              {t("ActionSettings")}
            </button>
            <button type="button" className="switcher__link switcher__link--muted" onClick={() => void quitApp().catch(() => {})}>
              {t("MenuQuit")}
            </button>
          </span>
        </footer>
      </div>
    </div>
  );
}

function AccountCard({
  row,
  busy,
  disabled,
  showAsUsed,
  now,
  locale,
  t,
  onSwitch,
}: {
  row: SwitcherRow;
  busy: boolean;
  disabled: boolean;
  showAsUsed: boolean;
  now: number;
  locale: string | undefined;
  t: ReturnType<typeof useLocale>["t"];
  onSwitch: () => void;
}) {
  const switchable = row.id !== null && !row.active;
  const className = [
    "switcher__card",
    row.active ? "switcher__card--active" : "",
    switchable ? "switcher__card--switchable" : "",
    busy ? "switcher__card--busy" : "",
  ]
    .filter(Boolean)
    .join(" ");
  const body = (
    <>
      <div className="switcher__card-head">
        <span className="switcher__check" aria-hidden="true">
          {row.active ? "✓" : ""}
        </span>
        <span className="switcher__email" title={row.title}>
          {row.title}
        </span>
        {row.plan && <span className="switcher__plan">{row.plan}</span>}
      </div>
      {row.lanes.map((lane) => (
        <LaneRow key={`${lane.kind}:${lane.windowMinutes}`} lane={lane} showAsUsed={showAsUsed} now={now} locale={locale} t={t} />
      ))}
      {row.note && <div className="switcher__note">{row.note}</div>}
    </>
  );
  if (!switchable) {
    return <div className={className}>{body}</div>;
  }
  return (
    <button
      type="button"
      className={className}
      disabled={disabled}
      onClick={onSwitch}
      title={`${t("CodexAccountsSwitchButton")}: ${row.title}`}
    >
      {body}
    </button>
  );
}

function LaneRow({
  lane,
  showAsUsed,
  now,
  locale,
  t,
}: {
  lane: SwitcherLane;
  showAsUsed: boolean;
  now: number;
  locale: string | undefined;
  t: ReturnType<typeof useLocale>["t"];
}) {
  const used = lane.usedPercent;
  const left = 100 - used;
  const display = showAsUsed ? used : left;
  const tone = used >= 90 ? "crit" : used >= 70 ? "warn" : "ok";
  const label =
    lane.kind === "session"
      ? t("ProviderSession")
      : lane.kind === "weekly"
        ? t("ProviderWeekly")
        : lane.windowMinutes
          ? `${Math.round(lane.windowMinutes / 1440)}d`
          : "";
  const countdown = resetCountdown(lane.resetsAt, now);
  const at = formatResetDateTime(lane.resetsAt, new Date(now), locale);
  const reset = countdown ? `${t("ResetsInShort")} ${countdown}${at ? ` • ${at}` : ""}` : null;
  const style = { "--fill": `${display}%` } as CSSProperties;
  return (
    <div className={`switcher__lane switcher__lane--${tone}`}>
      <div className="switcher__lane-label">
        {label}
        <span className={`switcher__dot switcher__dot--${tone}`} aria-hidden="true" />
      </div>
      <div className="switcher__bar" style={style} aria-hidden="true">
        <span />
      </div>
      <div className="switcher__lane-meta">
        <span className="switcher__pct">
          {Math.round(display)}% {showAsUsed ? t("PanelUsedSuffix") : t("PanelLeftSuffix")}
        </span>
        {reset && <span className="switcher__reset">{reset}</span>}
      </div>
    </div>
  );
}
