# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

SlackWatch is a Kubernetes tool that monitors container image versions and automates updates via GitOps. It watches pods annotated with `slackwatch.enable: true`, compares their image tags against container registries using semver, sends ntfy notifications when updates are available, and can automatically commit new image tags to a GitOps repository.

## Architecture

**Dual-component application** — a Rust backend and a React frontend, deployed as a single binary serving embedded static files.

```
┌─────────────────────────────────────────────────────────────┐
│                        Docker Container                      │
│  ┌───────────────────────────────────────────────────────┐  │
│  │  Rust Backend (warp + tokio) — port 8080             │  │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────┐ │  │
│  │  │  API     │ │ Scheduler│ │ Services │ │  GitOps │ │  │
│  │  │ (warp)   │ │ (cron)   │ │(workloads)│ │ (git2)  │ │  │
│  │  └────┬─────┘ └────┬─────┘ └────┬─────┘ └────┬────┘ │  │
│  │       └────────────┴────────────┴────────────┘      │  │
│  │                          │                           │  │
│  │  ┌───────────────────────┴───────────────────────┐  │  │
│  │  │           Core Modules                        │  │  │
│  │  │  kubernetes (kube crate)                      │  │  │
│  │  │  repocheck (oci_distribution for registry)    │  │  │
│  │  │  notifications (ntfy)                         │  │  │
│  │  │  database (rusqlite, local SQLite)            │  │  │
│  │  │  models (Workload, UpdateStatus)              │  │  │
│  │  │  config (config crate: TOML/env vars)         │  │  │
│  │  └───────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────────────────┐  │
│  │  Frontend (Vite + React) — static files served by    │  │
│  │  Rust backend from frontend/dist/                     │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

### Key modules (`src/`)

| Module | Purpose |
|--------|---------|
| `main.rs` | Entry point — initializes settings, creates SQLite DB, spawns scheduler, starts API server |
| `api.rs` | REST API routes (warp filters) — workloads CRUD, settings, schedule info |
| `config.rs` | Settings loading from TOML file, YAML file, or `SLACKWATCH_*` env vars |
| `kubernetes/client.rs` | Lists pods via kube crate, extracts workloads from pod annotations |
| `repocheck/repocheck.rs` | Fetches image tags from OCI registries via oci_distribution |
| `services/workloads.rs` | Core logic: tag parsing, semver comparison, include/exclude regex filtering |
| `services/scheduler.rs` | Cron-based scheduler (cron crate) that periodically refreshes all workloads |
| `gitops/gitops.rs` | Clones GitOps repo, edits YAML manifests (Deployment/StatefulSet), commits and pushes |
| `notifications/ntfy.rs` | Sends notifications via ntfy (update available + commit confirmation) |
| `database/client.rs` | SQLite persistence via rusqlite — stores scan history |
| `models/models.rs` | Domain types: `Workload`, `UpdateStatus`, `ApiResponse` |

### Workload discovery

Workloads are discovered from Kubernetes pods. A pod is watched if it has the annotation `slackwatch.enable: true`. Additional annotations control behavior:
- `slackwatch.include` — comma-separated regex patterns for including specific tags
- `slackwatch.exclude` — comma-separated regex patterns for excluding tags
- `slackwatch.repo` — name matching a `[[gitops]]` config entry for auto-upgrade
- `slackwatch.directory` — subdirectory in the GitOps repo to search

### Configuration

Settings are loaded via the `config` crate with this priority (highest first):
1. `SLACKWATCH_*` environment variables
2. File specified by `SLACKWATCH_CONFIG` env var (TOML)
3. `.env.yaml` file
4. `/app/config/config` file

See `docs/configuration.md` for full config reference.

## Build Commands

### Backend (Rust)
```bash
# Build
cargo build

# Run (development)
cargo run

# Run tests
cargo test

# Run a specific test
cargo test test_settings_load_success

# Clippy linting
cargo clippy --all-features

# Format
cargo fmt
```

### Frontend (React + Vite + TypeScript)
```bash
cd frontend

# Install dependencies
npm install

# Development server (hot reload, proxies API to backend)
npm run dev

# Build for production
npm run build

# Preview production build
npm run preview
```

### Docker
```bash
# Build full image (multi-stage: frontend + backend)
docker build -t slackwatch .

# Run
docker run -p 8080:8080 slackwatch
```

### Documentation
```bash
# Build mdBook docs (requires mdbook)
mdbook build
```

## Development Notes

- The scheduler runs on a cron schedule (default: every 2 hours between 9am-10pm). Set `system.run_at_startup = true` to trigger a refresh on startup.
- The frontend dev server (`npm run dev`) proxies `/api` requests to the backend on port 8080.
- The backend serves the React SPA from `frontend/dist/` — production builds require `npm run build` first.
- GitOps operations clone repos to `/tmp/repos/<name>`, edit YAML files matching Deployment/StatefulSet specs, then commit and push to `refs/heads/main`.
- Image tag fetching uses OCI distribution protocol with pagination support (up to 20 attempts × 1500 tags).
- The database stores full scan history (not just latest), enabling trend tracking across scans.
