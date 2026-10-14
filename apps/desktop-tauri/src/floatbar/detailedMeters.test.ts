import { describe, expect, it } from "vitest";
import {
  detailedLanes,
  formatResetDateTime,
  laneTone,
  resetCountdown,
  windowShortLabel,
} from "./detailedMeters";
import type { ProviderUsageSnapshot, RateWindowSnapshot } from "../types/bridge";

function win(used: number, windowMinutes: number | null, extra: Partial<RateWindowSnapshot> = {}): RateWindowSnapshot {
  return {
    usedPercent: used,
    remainingPercent: 100 - used,
    windowMinutes,
    resetsAt: null,
    resetDescription: null,
    isExhausted: false,
    reservePercent: null,
    reserveDescription: null,
    ...extra,
  };
}

function provider(overrides: Partial<ProviderUsageSnapshot>): ProviderUsageSnapshot {
  const primary = win(10, 300);
  return {
    providerId: "claude",
    displayName: "Claude",
    primary,
    selectedMetric: primary,
    secondary: null,
    modelSpecific: null,
    tertiary: null,
    extraRateWindows: [],
    cost: null,
    planName: null,
    accountEmail: null,
    sourceLabel: "oauth",
    updatedAt: "2026-10-15T00:00:00Z",
    error: null,
    errorState: "ready",
    pace: null,
    accountOrganization: null,
    trayStatusLabel: null,
    ...overrides,
  };
}

const thresholds = {
  highUsageThreshold: 70,
  criticalUsageThreshold: 90,
} as Parameters<typeof laneTone>[0];

describe("windowShortLabel", () => {
  it("labels hours, days and minutes", () => {
    expect(windowShortLabel(300, "S")).toBe("5h");
    expect(windowShortLabel(10080, "W")).toBe("7d");
    expect(windowShortLabel(43200, "M")).toBe("30d");
    expect(windowShortLabel(90, "X")).toBe("90m");
  });

  it("falls back when the window length is unknown", () => {
    expect(windowShortLabel(null, "Session")).toBe("Session");
    expect(windowShortLabel(0, "W")).toBe("W");
  });
});

describe("detailedLanes", () => {
  it("returns session and weekly lanes", () => {
    const lanes = detailedLanes(provider({ secondary: win(40, 10080) }));
    expect(lanes.map((l) => [l.lane, l.label])).toEqual([
      ["primary", "5h"],
      ["secondary", "7d"],
    ]);
  });

  it("uses tertiary when there is no secondary", () => {
    const lanes = detailedLanes(provider({ tertiary: win(40, 10080) }));
    expect(lanes.map((l) => l.lane)).toEqual(["primary", "tertiary"]);
  });

  it("skips informational windows", () => {
    const lanes = detailedLanes(
      provider({ primary: win(0, null, { isInformational: true }), secondary: win(5, 10080) }),
    );
    expect(lanes.map((l) => l.lane)).toEqual(["secondary"]);
  });

  it("is empty with no quota windows", () => {
    expect(detailedLanes(provider({ primary: win(0, null, { isInformational: true }) }))).toEqual([]);
  });
});

describe("laneTone", () => {
  it("follows the high and critical thresholds", () => {
    expect(laneTone(thresholds, "claude", "primary", win(50, 300))).toBe("ok");
    expect(laneTone(thresholds, "claude", "primary", win(75, 300))).toBe("warn");
    expect(laneTone(thresholds, "claude", "secondary", win(92, 10080))).toBe("crit");
    expect(laneTone(thresholds, "claude", "primary", win(10, 300, { isExhausted: true }))).toBe("crit");
  });
});

describe("formatResetDateTime", () => {
  const now = new Date(2026, 9, 15, 10, 0); // Thu Oct 15 2026, local time

  it("shows only the time for today", () => {
    const at = new Date(2026, 9, 15, 14, 30).toISOString();
    expect(formatResetDateTime(at, now, "en-GB")).toBe("14:30");
  });

  it("adds the weekday within the week", () => {
    const at = new Date(2026, 9, 19, 9, 5).toISOString();
    expect(formatResetDateTime(at, now, "en-GB")).toBe("Mon 09:05");
  });

  it("adds the date further out", () => {
    const at = new Date(2026, 10, 2, 9, 5).toISOString();
    expect(formatResetDateTime(at, now, "en-GB")).toBe("2 Nov 09:05");
  });

  it("returns null without a timestamp", () => {
    expect(formatResetDateTime(null, now)).toBeNull();
    expect(formatResetDateTime("garbage", now)).toBeNull();
  });
});

describe("resetCountdown", () => {
  it("formats a countdown", () => {
    const now = Date.UTC(2026, 9, 15, 10, 0);
    expect(resetCountdown(new Date(now + (3 * 60 + 12) * 60_000).toISOString(), now)).toBe("3h 12m");
    expect(resetCountdown(new Date(now + (2 * 24 + 4) * 3_600_000).toISOString(), now)).toBe("2d 4h");
    expect(resetCountdown(new Date(now - 1000).toISOString(), now)).toBe("now");
  });
});
