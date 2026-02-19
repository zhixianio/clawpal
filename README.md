# ClawPal

A desktop companion app for [OpenClaw](https://github.com/openclaw/openclaw) — manage your AI agents, models, and configurations with a visual interface instead of editing JSON by hand.

## Features

- **Recipes** — Browse and apply pre-built configuration templates with parameter forms, live diffs, and automatic rollback on failure
- **Agent Management** — Create, configure, and monitor your OpenClaw agents at a glance
- **Model Profiles** — Set up API keys, browse the model catalog, and switch the global default model in one click
- **Channel Bindings** — Connect Discord channels to agents with per-channel model overrides
- **Doctor** — Run diagnostics, auto-fix common issues, and clean up stale sessions
- **History & Rollback** — Every config change is snapshotted; roll back to any point in time
- **Remote Management** — Connect to remote OpenClaw instances over SSH and manage them the same way
- **Auto-Update** — ClawPal checks for new versions and updates itself in-app

## Install

Download the latest release for your platform from [GitHub Releases](https://github.com/zhixianio/clawpal/releases):

| Platform | Format |
|----------|--------|
| macOS (Apple Silicon) | `.dmg` |
| macOS (Intel) | `.dmg` |
| Windows | `.exe` installer or portable |
| Linux | `.deb` / `.AppImage` |

## Development

Prerequisites: [Node.js](https://nodejs.org/) 20+, [Rust](https://www.rust-lang.org/tools/install), and [Tauri CLI](https://v2.tauri.app/start/prerequisites/)

```bash
npm install
npm run dev          # Vite dev server + Tauri window
```

### Build

```bash
npm run build
cd src-tauri && cargo build
```

### Release

```bash
npm run release:dry-run   # Preview version bump + tag
npm run release           # Tag and push (triggers CI)
```

### Environment overrides

```bash
export CLAWPAL_OPENCLAW_DIR="$HOME/.openclaw"   # OpenClaw config directory (default)
export CLAWPAL_DATA_DIR="$HOME/.clawpal"        # ClawPal metadata directory
```

## WSL2 (Windows Subsystem for Linux)

If you have OpenClaw installed inside WSL2, you can manage it from ClawPal using the built-in SSH Remote feature:

1. Enable SSH inside your WSL2 distro:
   ```bash
   sudo apt install openssh-server
   sudo systemctl enable ssh
   sudo systemctl start ssh
   ```

2. In ClawPal, add a new SSH host:
   - **Host**: `localhost`
   - **Port**: the SSH port (default `22`, or check with `ss -tlnp | grep ssh`)
   - **User**: your WSL2 username

3. Connect — ClawPal will manage the WSL2 OpenClaw instance the same as any remote server.

## Tech stack

- **Frontend** — React, TypeScript, Tailwind CSS, Radix UI
- **Backend** — Rust, Tauri 2
- **Remote** — russh (SSH/SFTP)

## Project layout

```
src/           React + TypeScript UI
src-tauri/     Rust + Tauri backend
docs/plans/    Design and implementation plans
```

## License

Proprietary. All rights reserved.
