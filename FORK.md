# AgentBar

AgentBar is an independent fork of [Win-CodexBar](https://github.com/nesszer/Win-CodexBar)
by NessZerra, which is the Windows port of [CodexBar](https://github.com/steipete/CodexBar)
by Peter Steinberger. It tracks the upstream fork, so it works with every provider
Win-CodexBar supports, and adds its own changes on top. Like the projects it builds
on, AgentBar is free and stays under the [MIT License](LICENSE). Credits and third-party
notices are in [NOTICE](NOTICE).

## What AgentBar changes

- **Tray left-click: account switcher.** A dark panel with every saved Claude Code and
  Codex account, its session and weekly usage, and reset times. Click an account to
  switch that provider; Claude Code sessions, Claude Desktop and Codex Desktop are
  closed and reopened on the new account. The full dashboard is on right-click.
- **Taskbar meter.** Session and weekly meters for the current accounts, docked on the
  Windows taskbar. Drag it left or right; right-click it to choose which monitor's
  taskbar it sits on (Settings → Menu → Taskbar Meter).
- **Floating bar detailed meters.** Optional two-row session/weekly tiles.
- **Soft dark dashboard** and a static gauge tray icon.
- **Branding:** the app is called AgentBar. Settings and saved accounts stay in the
  same folders as Win-CodexBar, so switching keeps your data.

## Staying in sync with upstream

`main` merges `nesszer/Win-CodexBar` `main` automatically
(`.github/workflows/sync-upstream.yml`, daily or on demand). A clean merge is pushed
and a new installer is built; a merge with conflicts opens an issue instead so it can
be resolved by hand. Branding is kept in `rust/src/brand.rs` and a few config files so
upstream merges stay small.

## Installing

Installers are published on this repository's
[Releases](https://github.com/jbrito-dev/Win-CodexBar/releases). They are not code-signed,
so Windows SmartScreen may warn on first run (More info → Run anyway). AgentBar installs
next to an existing CodexBar; uninstall CodexBar afterwards so only one starts at login.
