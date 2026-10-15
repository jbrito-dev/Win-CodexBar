/**
 * Pure model for the tray account switcher: turns the Claude and Codex account
 * stores (plus the live provider snapshots for the active accounts) into one
 * row per account with up to two usage lanes.
 */
import type {
  ClaudeAccount,
  CodexAccountsStateBridge,
  CodexUsageWindow,
  ProviderUsageSnapshot,
  RateWindowSnapshot,
} from "../types/bridge";

export type SwitcherProvider = "claude" | "codex";

export interface SwitcherLane {
  kind: "session" | "weekly" | "other";
  /** Window length in minutes, when known. */
  windowMinutes: number | null;
  usedPercent: number;
  resetsAt: string | null;
}

export interface SwitcherRow {
  /** Account id used for switching; null when the row cannot be switched to. */
  id: string | null;
  provider: SwitcherProvider;
  title: string;
  plan: string | null;
  active: boolean;
  lanes: SwitcherLane[];
  /** Short status line shown instead of / below the lanes. */
  note: string | null;
  needsAuthentication: boolean;
}

const SESSION_MAX_MINUTES = 5 * 60;
const WEEK_MINUTES = 7 * 24 * 60;

function laneKind(windowMinutes: number | null, fallback: SwitcherLane["kind"]): SwitcherLane["kind"] {
  if (windowMinutes == null) return fallback;
  if (windowMinutes <= SESSION_MAX_MINUTES) return "session";
  if (windowMinutes === WEEK_MINUTES) return "weekly";
  return "other";
}

function clamp(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : 0;
}

function fromRateWindow(
  window: RateWindowSnapshot | null | undefined,
  fallback: SwitcherLane["kind"],
): SwitcherLane | null {
  if (!window || window.isInformational) return null;
  return {
    kind: laneKind(window.windowMinutes, fallback),
    windowMinutes: window.windowMinutes,
    usedPercent: clamp(window.usedPercent),
    resetsAt: window.resetsAt,
  };
}

function fromCodexWindow(
  window: CodexUsageWindow | null | undefined,
  fallback: SwitcherLane["kind"],
): SwitcherLane | null {
  if (!window) return null;
  const minutes = window.limitWindowSeconds > 0 ? Math.round(window.limitWindowSeconds / 60) : null;
  return {
    kind: laneKind(minutes, fallback),
    windowMinutes: minutes,
    usedPercent: clamp(window.usedPercent),
    resetsAt: window.resetAt,
  };
}

/** Lanes from a live provider snapshot (the active account). */
export function snapshotLanes(snapshot: ProviderUsageSnapshot | undefined): SwitcherLane[] {
  if (!snapshot || snapshot.errorState !== "ready") return [];
  return [
    fromRateWindow(snapshot.primary, "session"),
    fromRateWindow(snapshot.secondary ?? snapshot.tertiary, "weekly"),
  ].filter((lane): lane is SwitcherLane => lane != null);
}

function sameEmail(a: string | null | undefined, b: string | null | undefined): boolean {
  return Boolean(a && b && a.trim().toLowerCase() === b.trim().toLowerCase());
}

export function claudeRows(
  accounts: ClaudeAccount[],
  snapshot: ProviderUsageSnapshot | undefined,
): SwitcherRow[] {
  const rows = accounts.map<SwitcherRow>((account) => {
    const stored: SwitcherLane[] = [];
    if (account.usage?.fiveHour) {
      stored.push({
        kind: "session",
        windowMinutes: SESSION_MAX_MINUTES,
        usedPercent: clamp(account.usage.fiveHour.usedPercent),
        resetsAt: account.usage.fiveHour.resetsAt,
      });
    }
    if (account.usage?.sevenDay) {
      stored.push({
        kind: "weekly",
        windowMinutes: WEEK_MINUTES,
        usedPercent: clamp(account.usage.sevenDay.usedPercent),
        resetsAt: account.usage.sevenDay.resetsAt,
      });
    }
    const live = account.isActive ? snapshotLanes(snapshot) : [];
    const lanes = live.length > 0 ? live : stored;
    return {
      id: account.isActive ? null : account.id,
      provider: "claude",
      title: account.email,
      plan: account.plan,
      active: account.isActive,
      lanes,
      note: lanes.length === 0 ? (account.usageError ?? null) : null,
      needsAuthentication: account.needsAuthentication === true,
    };
  });
  if (rows.length === 0 && snapshot) {
    rows.push(activeOnlyRow("claude", snapshot));
  }
  return sortRows(rows);
}

export function codexRows(
  state: CodexAccountsStateBridge | null,
  snapshot: ProviderUsageSnapshot | undefined,
): SwitcherRow[] {
  const accounts = state?.accounts ?? [];
  const activeId =
    state?.activeAccountId ??
    accounts.find((account) =>
      sameEmail(state?.snapshots[account.id]?.email ?? account.emailHint, snapshot?.accountEmail),
    )?.id ??
    null;
  const rows = accounts.map<SwitcherRow>((account) => {
    const usage = state?.snapshots[account.id];
    const active = account.id === activeId;
    const stored = usage
      ? [fromCodexWindow(usage.primaryWindow, "session"), fromCodexWindow(usage.secondaryWindow, "weekly")].filter(
          (lane): lane is SwitcherLane => lane != null,
        )
      : [];
    const live = active ? snapshotLanes(snapshot) : [];
    const lanes = live.length > 0 ? live : stored;
    const title =
      state?.displayNames?.[account.id] ?? usage?.email ?? account.emailHint ?? account.nickname ?? "Codex";
    return {
      id: active ? null : account.id,
      provider: "codex",
      title,
      plan: usage?.plan ?? (active ? snapshot?.planName ?? null : null),
      active,
      lanes,
      note: null,
      needsAuthentication: state?.needsAuthentication?.[account.id] === true,
    };
  });
  if (rows.length === 0 && snapshot) {
    rows.push(activeOnlyRow("codex", snapshot));
  }
  return sortRows(rows);
}

function activeOnlyRow(provider: SwitcherProvider, snapshot: ProviderUsageSnapshot): SwitcherRow {
  return {
    id: null,
    provider,
    title: snapshot.accountEmail ?? snapshot.displayName,
    plan: snapshot.planName,
    active: true,
    lanes: snapshotLanes(snapshot),
    note: snapshot.errorState === "ready" ? null : snapshot.error,
    needsAuthentication: snapshot.errorState === "needsAuthentication",
  };
}

/** Active account first, then the rest in their stored order. */
function sortRows(rows: SwitcherRow[]): SwitcherRow[] {
  return [...rows.filter((row) => row.active), ...rows.filter((row) => !row.active)];
}
