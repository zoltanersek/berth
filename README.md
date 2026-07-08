# Berth

**Give every AI coding agent its own isolated dev environment — no more port collisions, no shared state, one command up and down.**

Running 5, 10, 50 coding agents in parallel on git worktrees? They step on each
other: same ports, same database, same cache. Berth gives each agent its own
**berth** — an isolated git worktree plus a disposable dev environment with
auto-assigned, non-colliding ports and its own state — and tears it all down
with one command.

> Local-first, single binary, free and open source. It's not an agent runner and
> not a cloud platform — it's the environment layer *underneath* whatever agents
> you already use (Claude Code, Codex, Cursor, Aider, …).

## What you get per berth

- An **isolated git worktree** on a fresh branch (`branch-per-agent`).
- **Auto-allocated free ports**, injected as env vars so nothing collides.
- **Isolated state** — its own Docker Compose project (network, volumes, DB,
  cache), namespaced per berth. Agents never share.
- **One command up, one command down.**
- A **live view** of every berth — in the terminal (`berth ls`) or a local web
  **dashboard** (`berth dashboard`).

## Requirements

- `git`
- `docker` with the Compose plugin (`docker compose`), daemon running

## Install

Build from source (Rust toolchain required):

```sh
cargo install --path .
# or
cargo build --release   # binary at target/release/berth
```

## Quickstart

Add a `berth.yml` to your repo (see [Configuration](#configuration)), then:

```sh
berth up agent-1      # worktree + services on fresh ports, prints URLs
berth up agent-2      # a second, fully isolated environment in parallel
berth ls              # every berth: branch, status, age, ports
berth down agent-1    # tear down services + volumes, remove worktree + branch
```

`berth up` prints the assigned ports and URLs:

```
Worktree:
  /path/to/myrepo-agent-1

Ports:
  BACKEND_PORT         60291
  FRONTEND_PORT        60292

URLs:
  http://localhost:60291
  http://localhost:60292
```

## Commands

| Command | Description |
| --- | --- |
| `berth up <name>` | Create a worktree on branch `<name>` and bring up its services with auto-assigned free ports and isolated volumes. |
| `berth down <name>` | Tear down services + volumes and remove the worktree. Refuses if the branch is unmerged or dirty (see `--force`). |
| `berth down <name> --force` | Tear down and **discard** the worktree even if unmerged/dirty — for abandoned agents. |
| `berth ls` | List every berth: name, branch, status, age, ports. |
| `berth dashboard` | Serve a local web dashboard with live env, ports, status and streaming logs, plus start/stop/restart/tear-down actions. |
| `berth agent <name> -- <cmd>` | Create a berth, run `<cmd>` inside its worktree, and safely tear it down when the command exits. |
| `berth start <name>` / `berth stop <name>` | Bring an existing berth's environment up / stop it (keeps the worktree). |
| `berth hooks install` | Wire berth into Claude Code / Codex so a berth's environment follows the agent session. |
| `berth snapshot save <name> [label]` | Capture a berth's data volumes as a named snapshot (default `baseline`). |
| `berth reset <name> [label]` | Reset a berth's volumes to a saved snapshot. |
| `berth validate` | Check that `berth.yml` and the referenced compose file are valid. |

All commands accept `--dir <path>` to target a repo other than the current
directory.

### Dashboard

`berth dashboard` serves a status page for every berth in the repo at
`http://127.0.0.1:<port>` — an auto-assigned free port by default, or pin one
with `--port`. It shows each berth's injected env, ports, status and
**live-streaming logs**, and can start, stop, restart, or tear a berth down from
the browser.

The server binds to loopback only and gates every request with a per-session
token, so it is not reachable from other machines. Pass `--no-open` to skip
opening a browser on startup.

### Agent integration

Bind a berth's lifecycle to a coding agent's. Two ways, use whichever fits:

**Launcher** — one command creates the berth, runs the agent inside its worktree,
and tears the berth down on exit:

```sh
berth agent my-feature -- claude      # or: codex, cursor, aider, …
```

Teardown on exit is a safe `berth down`: it removes the worktree+branch only if
the branch is **merged and clean**, and otherwise keeps it (run `berth down
--force` to discard abandoned work). This is the most reliable option and works
with any agent.

**Hooks** — if you launch the agent yourself, install hooks so the environment
follows the session:

```sh
berth hooks install            # Claude Code + Codex; --claude / --codex to pick one, --global for ~/
```

Session start brings the current worktree's environment up; session end runs the
same safe teardown (skipping `clear`/`resume`, which only replace the session).
Codex has no session-end event, so on Codex the end side relies on the launcher
above. Remove with `berth hooks uninstall`.

### Snapshots & reset

Capture a berth's data volumes at a known baseline and roll any berth back to it
— for reproducible starting state and fast recovery from a wrecked database:

```sh
berth snapshot save api baseline   # capture api's volumes
berth reset api baseline           # …later, restore them (or just `berth reset api`)
berth up api2 --seed baseline      # start a fresh berth pre-seeded with that data
```

Snapshots are engine-agnostic (they tar the Docker volumes, so any service
works) and keyed by label, so a baseline captured from one berth can seed
another. The berth's containers are stopped briefly during capture/restore for a
consistent copy — fast, not literally instant. Snapshots live under the
gitignored `.berth/`, so they're local, not a committed team seed. `berth
snapshot list` / `berth snapshot rm <label>` manage them; add
`snapshot: { volumes: [...] }` to `berth.yml` to capture only some volumes.

## Configuration

Berth reads a `berth.yml` at the root of your repo:

```yaml
version: 1

# Path to your Docker Compose file (relative to the repo root).
compose: docker-compose.yml

# Ports Berth should auto-allocate. For each entry it finds a free port and
# injects it into the environment under the given variable name.
ports:
  frontend:
    env: FRONTEND_PORT
  backend:
    env: BACKEND_PORT
```

Reference those variables from your compose file so each berth binds to its own
free ports:

```yaml
services:
  frontend:
    build: ./frontend
    ports:
      - "${FRONTEND_PORT}:${FRONTEND_PORT}"
    environment:
      FRONTEND_PORT: ${FRONTEND_PORT}
      BERTH_NAME: ${BERTH_NAME}
```

Berth also injects `BERTH_NAME` and `COMPOSE_PROJECT_NAME` (`berth-<name>`) so
container names, networks, and volumes are namespaced per berth.

A complete, runnable example lives in [`hello-berth/`](./hello-berth).

## How it works

- **Worktree**: `git worktree add -b <name>` next to your repo.
- **Ports**: each declared port is bound to `127.0.0.1:0` so the OS hands back a
  free one; all are held until every port is chosen (so none collide), then
  written to `.berth/<name>.env`.
- **Environment**: `docker compose -p berth-<name> --env-file .berth/<name>.env up -d`
  — the per-project name isolates networks and volumes.
- **State**: berths are tracked in `.berth/state.json`, guarded by an advisory
  file lock so concurrent `up`/`down` runs (the whole point) don't clobber it.

Everything Berth writes lives under `.berth/`, which it adds to your
`.gitignore` automatically.

## License

[MIT](./LICENSE)
