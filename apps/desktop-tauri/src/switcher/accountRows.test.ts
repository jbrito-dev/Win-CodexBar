import { describe, expect, it } from "vitest";
import { claudeRows, codexRows } from "./accountRows";
import type { ClaudeAccount, CodexAccountsStateBridge, ProviderUsageSnapshot, RateWindowSnapshot } from "../types/bridge";

function win(used: number, minutes: number, resetsAt: string | null = null): RateWindowSnapshot {
  return {
    usedPercent: used,
    remainingPercent: 100 - used,
    windowMinutes: minutes,
    resetsAt,
    resetDescription: null,
    isExhausted: false,
    reservePercent: null,
    reserveDescription: null,
  };
}

function snapshot(id: string, email: string, primary: number, secondary: number): ProviderUsageSnapshot {
  const p = win(primary, 300);
  return {
    providerId: id,
    displayName: id,
    primary: p,
    selectedMetric: p,
    secondary: win(secondary, 10080),
    modelSpecific: null,
    tertiary: null,
    extraRateWindows: [],
    cost: null,
    planName: "plus",
    accountEmail: email,
    sourceLabel: "oauth",
    updatedAt: "2026-10-15T00:00:00Z",
    error: null,
    errorState: "ready",
    pace: null,
    accountOrganization: null,
    trayStatusLabel: null,
  };
}

describe("claudeRows", () => {
  const accounts: ClaudeAccount[] = [
    { id: "b", email: "b@x.com", organization: null, plan: "pro", isActive: false, isSaved: true,
      usage: { fiveHour: { usedPercent: 10, resetsAt: null }, sevenDay: { usedPercent: 50, resetsAt: null }, updatedAt: "" } },
    { id: "a", email: "a@x.com", organization: null, plan: "max", isActive: true, isSaved: true, usage: null },
  ];

  it("puts the active account first with live usage and makes others switchable", () => {
    const rows = claudeRows(accounts, snapshot("claude", "a@x.com", 42, 18));
    expect(rows.map((r) => r.title)).toEqual(["a@x.com", "b@x.com"]);
    expect(rows[0].active).toBe(true);
    expect(rows[0].id).toBeNull();
    expect(rows[0].lanes.map((l) => [l.kind, l.usedPercent])).toEqual([["session", 42], ["weekly", 18]]);
    expect(rows[1].id).toBe("b");
    expect(rows[1].lanes.map((l) => l.usedPercent)).toEqual([10, 50]);
  });

  it("falls back to the provider snapshot when no accounts are saved", () => {
    const rows = claudeRows([], snapshot("claude", "a@x.com", 1, 2));
    expect(rows).toHaveLength(1);
    expect(rows[0].active).toBe(true);
  });
});

describe("codexRows", () => {
  const state: CodexAccountsStateBridge = {
    accounts: [
      { id: "1", nickname: null, emailHint: "one@x.com", authSubject: null, providerAccountId: null, codexHomePath: "", source: "managedByApp", createdAt: "", updatedAt: "", lastAuthenticatedAt: null },
      { id: "2", nickname: null, emailHint: "two@x.com", authSubject: null, providerAccountId: null, codexHomePath: "", source: "ambient", createdAt: "", updatedAt: "", lastAuthenticatedAt: null },
    ],
    accountOrdinals: {},
    snapshots: {
      "1": { email: "one@x.com", providerAccountId: null, plan: "plus", allowed: true, limitReached: false,
        primaryWindow: { usedPercent: 1, resetAt: null, limitWindowSeconds: 18000 },
        secondaryWindow: { usedPercent: 31, resetAt: null, limitWindowSeconds: 604800 }, credits: null, updatedAt: "" },
    },
    activeAccountId: "2",
  };

  it("marks the ambient account active and maps stored windows", () => {
    const rows = codexRows(state, snapshot("codex", "two@x.com", 0, 0));
    expect(rows[0].title).toBe("two@x.com");
    expect(rows[0].active).toBe(true);
    const other = rows[1];
    expect(other.id).toBe("1");
    expect(other.lanes.map((l) => [l.kind, l.usedPercent])).toEqual([["session", 1], ["weekly", 31]]);
  });
});
