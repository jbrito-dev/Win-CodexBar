/**
 * Pure helpers for the float bar's "Detailed meters" mode.
 *
 * In detailed mode each provider gets one tile with up to two mini meters: the
 * session lane (primary, usually the 5-hour window) and the weekly lane
 * (secondary, or tertiary when a provider only reports that one), each with an
 * absolute reset date/time. Kept free of React so the lane selection and the
 * date formatting can be unit-tested directly.
 */
import { type MetricLane, quotaWarningWindowForLane } from "../lib/metricRowModel";
import { type QuotaWarningSettings, resolveQuotaWarning } from "../lib/quotaWarnings";
import type { ProviderUsageSnapshot, RateWindowSnapshot } from "../types/bridge";

export type MeterTone = "ok" | "warn" | "crit";

export interface DetailedLane {
  lane: MetricLane;
  window: RateWindowSnapshot;
  /** Short lane label, e.g. "5h" or "7d". */
  label: string;
}

const MINUTES_PER_HOUR = 60;
const MINUTES_PER_DAY = 24 * MINUTES_PER_HOUR;

/**
 * Compact label for a window length: "5h", "7d", "30d", "90m". Falls back to
 * the provider's own lane label when the window length is unknown.
 */
export function windowShortLabel(windowMinutes: number | null, fallback: string): string {
  if (windowMinutes == null || !Number.isFinite(windowMinutes) || windowMinutes <= 0) {
    return fallback;
  }
  if (windowMinutes % MINUTES_PER_DAY === 0) return `${windowMinutes / MINUTES_PER_DAY}d`;
  if (windowMinutes % MINUTES_PER_HOUR === 0) return `${windowMinutes / MINUTES_PER_HOUR}h`;
  return `${Math.round(windowMinutes)}m`;
}

function isQuotaWindow(window: RateWindowSnapshot | null | undefined): window is RateWindowSnapshot {
  return window != null && window.isInformational !== true;
}

/**
 * The lanes a detailed tile shows: the session lane and the weekly lane.
 * Informational windows (no budget, balance lines) carry no percentage and are
 * skipped. Returns an empty list when the provider has no quota window, so the
 * caller can fall back to the compact pill.
 */
export function detailedLanes(provider: ProviderUsageSnapshot): DetailedLane[] {
  const lanes: DetailedLane[] = [];
  if (isQuotaWindow(provider.primary)) {
    lanes.push({
      lane: "primary",
      window: provider.primary,
      label: windowShortLabel(provider.primary.windowMinutes, provider.primaryLabel?.trim() || "S"),
    });
  }
  if (isQuotaWindow(provider.secondary)) {
    lanes.push({
      lane: "secondary",
      window: provider.secondary,
      label: windowShortLabel(
        provider.secondary.windowMinutes,
        provider.secondaryLabel?.trim() || "W",
      ),
    });
  } else if (isQuotaWindow(provider.tertiary)) {
    lanes.push({
      lane: "tertiary",
      window: provider.tertiary,
      label: windowShortLabel(provider.tertiary.windowMinutes, provider.tertiaryLabel?.trim() || "W"),
    });
  }
  return lanes;
}

/** Usage tone for one lane, following the same quota warning thresholds as the cards. */
export function laneTone(
  thresholds: QuotaWarningSettings,
  providerId: string,
  lane: MetricLane,
  window: RateWindowSnapshot,
): MeterTone {
  const warningWindow = quotaWarningWindowForLane(lane);
  const warning = warningWindow
    ? resolveQuotaWarning(thresholds, providerId, warningWindow)
    : { enabled: true, high: thresholds.highUsageThreshold, critical: thresholds.criticalUsageThreshold };
  const used = clampPercent(window.usedPercent);
  if (window.isExhausted) return "crit";
  if (!warning.enabled) return "ok";
  if (used >= warning.critical) return "crit";
  if (used >= warning.high) return "warn";
  return "ok";
}

export function clampPercent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, value));
}

function sameLocalDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate()
  );
}

/**
 * Compact absolute reset date/time in local time:
 *  - today → "14:30"
 *  - within the next 6 days → "Thu 14:30"
 *  - later → "Oct 21 14:30"
 * Returns null when `resetsAt` is missing or unparseable.
 */
export function formatResetDateTime(
  resetsAt: string | null,
  now: Date,
  locale?: string,
): string | null {
  if (!resetsAt) return null;
  const target = new Date(Date.parse(resetsAt));
  if (Number.isNaN(target.getTime())) return null;

  const time = new Intl.DateTimeFormat(locale, { hour: "2-digit", minute: "2-digit" }).format(target);
  if (sameLocalDay(target, now)) return time;

  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const daysAhead = Math.floor((target.getTime() - startOfToday) / (MINUTES_PER_DAY * 60_000));
  if (daysAhead >= 0 && daysAhead < 7) {
    const weekday = new Intl.DateTimeFormat(locale, { weekday: "short" }).format(target);
    return `${weekday} ${time}`;
  }
  const date = new Intl.DateTimeFormat(locale, { month: "short", day: "numeric" }).format(target);
  return `${date} ${time}`;
}

/** Full reset date/time for the tooltip, e.g. "Thu, Oct 16, 14:30". */
export function formatResetDateTimeLong(
  resetsAt: string | null,
  locale?: string,
): string | null {
  if (!resetsAt) return null;
  const target = new Date(Date.parse(resetsAt));
  if (Number.isNaN(target.getTime())) return null;
  return new Intl.DateTimeFormat(locale, {
    weekday: "short",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(target);
}

/** Short countdown for the tooltip: "3h 12m", "2d 4h", "now". */
export function resetCountdown(resetsAt: string | null, nowMs: number): string | null {
  if (!resetsAt) return null;
  const target = Date.parse(resetsAt);
  if (Number.isNaN(target)) return null;
  const diffMs = target - nowMs;
  if (diffMs <= 0) return "now";
  const totalMinutes = Math.max(1, Math.floor(diffMs / 60_000));
  const days = Math.floor(totalMinutes / MINUTES_PER_DAY);
  const hours = Math.floor((totalMinutes % MINUTES_PER_DAY) / MINUTES_PER_HOUR);
  const minutes = totalMinutes % MINUTES_PER_HOUR;
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}
