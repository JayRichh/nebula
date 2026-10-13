# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

<!-- The GUI is a Tauri desktop app (web view). The terminal TUI (ratatui) is a second surface over the same daemon. -->

## Stack

- Rust workspace forked from nebula (MIT, AgentSystemLabs): daemon + ratatui TUI.
- Added: a Tauri v2 desktop GUI over the daemon's IPC.
- Windows-native first; macOS and Linux stay supported from the same codebase.

## Users

Developers who run several AI coding agents in parallel across projects and git worktrees, and need to know at a glance which agent needs them. Public, open source from day one; the first user is a Windows 11 developer with two Claude Max subscriptions, a ChatGPT subscription (Codex) and GitHub Copilot.

## Product Purpose

Bro is mission control for coding agents on Windows: one daemon owns every agent session (Claude Code, Codex, Copilot, Cursor, OpenCode, Pi, Grok, Muse), sessions survive closing the UI, and every session's state is readable without opening it. Success: you never tab through terminals to find which agent is waiting on you.

## Positioning

- Native Windows: no WSL, no tmux.
- A local daemon that owns the PTYs, with nebula's keyboard-first workflow.
- Multi-account profiles: several subscriptions per harness, isolated config dirs, switch and resume.
- Several worktrees running dev servers side by side, with ports allocated and visible per worktree.

## Operating Context

- Windows Terminal, the VS Code integrated terminal, or Bro's own desktop window.
- PowerShell 7 / 5.1, Git Bash optional.
- `git`, `gh`, `git worktree`.
- Dev servers on localhost (Next.js, Vite and similar), opened in Chrome, Edge or Firefox.
- Works next to VS Code. The user's VS Code uses Atom One Dark, JetBrains Mono, and Material Icon Theme; Bro should feel at home beside it.

## Capabilities and Constraints

- **Inherited from nebula:**
  - session cards with status dots (running / unseen / done / needs input)
  - project tabs, worktree bands, quick prompt, orchestrator chat
  - diff viewer, file finder, grep, tree browser
  - PR and issue views via `gh`
  - harness table configured from `config.json`
- **New in Bro:**
  - **Profiles:** accounts per harness (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GH_CONFIG_DIR`).
  - **Kanban:** layout by status.
  - **Diff:** VS Code-grade, with side-by-side, inline and replacement modes, and per-hunk accept/reject.
  - **File icons:** in the explorer, matching the active theme.
  - **Theme:** light and dark awareness.
  - **Localhost previews:**
    - ports per worktree
    - port allocation per worktree
    - open in a chosen browser (Chrome, Edge or Firefox, with a browser profile each)
    - headless screenshots shown inline
  - **Copilot:** as a harness.
- **CLI:** `bro`. The command `bro` must not collide with nebula's `nebula`; config lives in `%LOCALAPPDATA%\bro`.
- **Optional:** agent-office runs alongside as a demo; it is not a dependency.

## Brand Commitments

- **Name:** Bro.
- **Voice:** Kiwi. "Bro" is camaraderie: a sidekick crew that has your back. Friendly, plain, never cutesy.
- **Tagline:** "Sweet as agent management."
- **Attribution:** keep nebula's MIT notice and credit.

## Evidence on Hand

- Upstream nebula README, docs, and screenshot (`assets/screenshot.png`) describe the inherited workflow.
- No users, testimonials, or benchmarks yet. Do not fabricate any.

## Product Principles

1. Status over screens: every session's state is readable without opening it.
2. Keyboard first, mouse welcome.
3. Native on Windows, honest everywhere: no WSL or tmux requirement, and macOS and Linux keep working.
4. Accounts are first-class: quota and identity are visible per session.
5. Never lose an agent: the daemon outlives every UI.

## Accessibility & Inclusion

- Status must not rely on colour alone: dots carry a glyph or label.
- Readable in both light and dark themes.
- Fully operable from the keyboard.
