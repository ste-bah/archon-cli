# Sandboxing

Archon's sandbox path is fail-closed. Permission mode, tool policy, and sandbox
backend selection are separate checks; provider spoofing and provider auth do
not change permission decisions.

## Commands

```bash
archon sandbox status --verbose
archon sandbox explain --backend docker
archon sandbox doctor --backend openshell
archon sandbox test --backend docker
```

Host Docker/OpenShell dependencies are optional. Install them with
`scripts/install-system-deps.sh --with-docker`, `--with-openshell`, or
`--with-sandbox`; then enable the matching backend in `[sandbox]`.

By default, `sandbox.mode = "risky"` routes shell execution through the selected
real backend but does not break normal host-side coding tools. `Write`, `Edit`,
`WebFetch`, and similar tools still pass through Archon's permission preflight.
Set `sandbox.mode = "all"` only when you want strict backend compatibility and
are willing to block unsupported host-side mutation, network, and agent-spawn
tools.

`mode` decides which tools are *gated* by the backend. It does not decide which
filesystem they operate on — that follows the backend in every mode, so under a
remote workspace `Write` writes the remote tree even on `risky`. See below.

## How long a sandbox lives

`sandbox.scope` decides how long one sandbox lives:

| `scope` | One sandbox per |
|---|---|
| `session` (default) | the whole run |
| `turn` | agent turn |
| `tool` | command |

This used to be validated, audited, printed, and read by nothing. Under
`backend = "docker"` a container was built and destroyed for every command
whatever it said. The bind mount covers only the workspace, so everything a
build leaves *beside* it — `~/.cargo/registry`, `~/.npm`, pip wheels, apt lists,
`/tmp` — went with the container. A sandboxed `cargo build` re-downloaded its
dependency graph on every call, and `workspace_access = "scratch"` was
meaningless because `/scratch` was destroyed by the command that wrote it.
Measured on the machine this was fixed on: 214ms per `docker run --rm` against
57ms per `docker exec`, and the latency was the small half.

**A sandbox is keyed by working directory as well as by scope.** That is not an
optimisation. A worktree-isolated subagent mounts a different tree and inherits
its parent's turn id, so one sandbox shared across the two would put two agents
in a single world while each believed it was isolated.

A caller with no turn loop — the workflow CLI, or any directly constructed tool
context — has no turn identity. Under `scope = "turn"` such a command gets its
own sandbox rather than sharing one with every other caller that also cannot
name its turn: two unrelated callers answering "no turn" have nothing in common,
and treating that as an identity would be the cross-agent leak the working
directory is in the key to prevent. The workflow CLI names its run as its turn,
so `turn` and `session` behave identically there and do so deliberately.

### What each backend does with it

Backends answer for themselves, as they do for terminals. A backend that cannot
honour a scope **fails config load** with the reason rather than quietly doing
something else:

| Backend | `session` | `turn` | `tool` |
|---|---|---|---|
| `docker` | one container held open, re-entered per command | one per turn, torn down at the boundary | one container per command |
| `ssh` | the remote host is durable — Archon neither creates nor destroys it, so every scope reaches the same machine and state always survives | as `session` | as `session` |
| `openshell` | **refused at config load** | **refused at config load** | one sandbox per command, which is what `--no-keep` does |
| `disabled`, `logical` | not applicable; no world is created | | |

`archon sandbox status` prints the answer as `Sandbox lifetime:`, so the
configured value and the actual behaviour are shown side by side.

**This is a breaking config change for `openshell`.** The default
`scope = "session"` no longer loads under `backend = "openshell"`; set
`scope = "tool"`, which is what that backend has always done. Whether OpenShell
can exec into an existing sandbox was not established — the CLI was not
available to test against — so claiming a longer lifetime for it would have been
a guess.

SSH connection multiplexing (`ControlMaster` + `ControlPersist`) is deliberately
not implemented. It would make the transport cheaper and would change nothing
about lifetime, since the remote host is already durable; no SSH target was
available to verify it against, and OpenSSH does not support `ControlMaster` on
Windows at all.

### Cleaning up held containers

Holding a container open trades away `--rm`'s guarantee, so the guarantee is
replaced by three independent mechanisms — independent because the first two do
not run in the case that matters most:

1. **Scope boundary.** A new turn tears down the previous turn's containers for
   that session; the session's containers are removed when the last reference to
   the backend is dropped. Best-effort by construction: `Drop` does not run under
   SIGKILL, an abort, or `std::process::exit`, and it does not run when something
   process-global still holds a reference — the workflow CLI installs its
   subagent executor as exactly that, so on that path teardown rests entirely on
   the two mechanisms below.
2. **Startup reaping.** Before the first container of a process is created,
   every container carrying Archon's labels whose creating process is gone is
   removed. Ownership is checked twice — a foreign owner id *and* a dead pid —
   because parallel Archon sessions on one machine are ordinary, and reaping on
   "not mine" alone would have two runs destroying each other's containers.
3. **The container's own age bound.** `sandbox.docker.container_max_age_secs`
   (default 4h) is the container's PID 1, and `--rm` is set, so it stops and is
   removed on its own. This is the only mechanism that needs nothing from the
   host, and it is what bounds the leak when Archon is killed and never started
   again. A command still running at that age is killed with the container, so
   the value must stay well above any Bash timeout; below 60s it is rejected.

A held container that has gone away for any of these reasons is rebuilt on the
next command rather than surfacing a daemon error. The daemon is asked whether
the container is running before anything is rebuilt, so an ordinary command
failure is never silently retried.

### What a held sandbox accumulates

A command that hits its Bash timeout has the local `docker` client killed, which
does not stop the process it started inside the container. Under a container per
command that process died with the container; under a held one it stays until the
container does, counting against that container's `--pids-limit`, memory and CPU
share for the rest of the scope. The container's age bound is the backstop.
Nothing tracks individual `exec` processes, so a scope full of timed-out commands
degrades rather than fails cleanly.

### Resource limits under a held sandbox

`--memory`, `--cpus` and `--pids-limit` are per container, and this change moves
the multiplier rather than removing it. A ten-agent fan-out sharing one working
directory used to run ten concurrent containers and now shares one, so the
limits apply once instead of ten times. A fan-out across ten *worktrees* still
gets ten containers, because they must not share a world — so the limits still
multiply there, exactly as before. There is no aggregate cap; that remains open.

## One world, not two

A sandbox that routes only `Bash` leaves the agent reading one filesystem and
executing against another. `Read`, `Glob` and `Grep` ran on the host while
`Bash` ran in the container or on the remote host, so the agent could grep
source, reason about what it found, and run commands against a different tree —
silently, with no warning anywhere.

The file tools now operate on the filesystem of the backend's world, whatever
`sandbox.mode` is set to. Which filesystem that is depends on the backend:

| Backend | Workspace | File tools operate on |
|---|---|---|
| `disabled`, `logical` | the host tree | the host |
| `docker` | bind mount of the working directory | the host bytes, with `/workspace/...` paths translated |
| `ssh` `mirror`, `openshell` `mirror` | assumed visible on both sides | the host |
| `ssh` `remote` | `remote_workdir` on the remote host | the remote workspace, over the same SSH transport `Bash` uses |
| `openshell` `remote` | `remote_workdir` or `/sandbox` | the remote workspace, over the OpenShell transport |
| `openshell` `upload` | re-uploaded per command | the host — see the note below |

**This changes behaviour for `ssh` in `remote` mode.** Previously `Write` and
`Edit` mutated the host while `Bash` ran on the remote target. They now write
the remote workspace, which is the tree `Bash` actually sees. If you were
relying on the old split — editing locally and executing remotely against a
different tree — use `workspace_mode = "mirror"` and keep the two in sync
yourself.

Read-before-edit freshness (`filesystem.read_before_edit`) is checked against
the same world, so under a remote workspace it now describes the file being
edited rather than a same-named file on the host.

If a backend's filesystem cannot be built, session boot fails rather than
falling back to the host. Falling back would silently restore the split.

### Which tools a backend will run

Backends used to decide by tool *name*, against a list of eight, with everything
unlisted denied. A tool added anywhere in Archon was then unusable under every
backend until three separate lists were updated, and nothing said so — the
terminal tools shipped broken exactly that way.

Each tool now declares what its effects reach, and the backends decide on that:

| Class | Under a real isolation backend |
|---|---|
| Archon's own state — memory, tasks, board, config | allowed; it is in the same place whatever the world is |
| Runs a command in the world | allowed, through the backend |
| Reads or writes the world's files | allowed, through the backend's filesystem |
| Opens an interactive terminal | the backend answers — see below |
| Reaches the world through a host handle it cannot redirect | refused, because it would run outside the sandbox |
| Leaves the machine | refused |
| Spawns or schedules work | refused |

A tool that declares nothing fails to compile, so the next one added is handled
on the day it is added rather than silently denied.

### Terminals

Under a real isolation backend a terminal either opens **in that world** or is
refused with the reason — it never quietly opens a host shell, which would be a
straight bypass of the boundary `Bash` is being routed through.

- **docker** attaches a TTY to a container built from the same arguments `Bash`
  uses, so the terminal sees the same workspace.
- **ssh** puts a TTY on the same connection, with the same strict options.
- **openshell** declines. It builds and destroys a sandbox per command, so there
  is no session to attach to, and one held open would be a different world from
  the next `Bash` call's. The refusal names docker as what would work.
- **`/sandbox on`** declines. It denies rather than relocating, so it has no
  world to put a shell in.

A docker terminal opens in a container of its own rather than in the container
`sandbox.scope` is holding. Both are built from the same arguments and both bind
the same workspace, so the terminal and `Bash` still agree about the tree — the
#201 property is unaffected. What they do not share is state *outside* the
workspace: a package the terminal installs, or a file it writes to `/tmp`, is not
what the next `Bash` call sees. `SandboxTerminal` is resolved synchronously and
carries no session or turn identity, so joining the held container would need
both; that is not done.

A terminal remembers the world it opened in. A host shell started before a
sandbox was switched on cannot be written to afterwards; close it and open a
new one.

### `openshell` `upload` is the exception

`upload` mode passes `--no-keep` and re-uploads the working directory on every
command, so each `Bash` call gets a fresh sandbox and anything it writes is
discarded when that sandbox exits. The host tree is the only durable copy and is
what the next command will be seeded from, so the file tools operate on the
host. Writes made by `Bash` inside an upload-mode sandbox are not visible to a
later `Read`, and cannot be — that is a property of the backend, not of the file
tools. Use `remote` mode if you need the two to agree.

## Backends

`logical` keeps the existing permission gate behavior.

`docker` can route Bash through a container when selected. It avoids mounting
host credentials, the Docker socket, or privileged host paths by default, and
uses the configured resource and network policy. The workspace is mounted
read-only unless `workspace_access = "rw"` is configured; teams can keep the
workspace read-only and expose only explicit relative `docker.writable_paths`.
`workspace_access = "scratch"` adds an ephemeral `/scratch` mount without
loosening the workspace.

`ssh` can route Bash through a configured remote target when selected. Remote
mode requires an explicit `ssh.remote_workdir`; mirror mode assumes the active
workspace path exists on the remote target. SSH does not forward provider
credentials, SSH agents, Git credentials, or arbitrary environment values, and
it refuses disabled host-key checking or host-shell fallback.

`openshell` can route Bash through `openshell sandbox create --no-keep --` when
selected. Archon does not pass `--provider`, does not forward request
environment values, strips common provider credential variables from the
OpenShell CLI process, and sets `OPENSHELL_GATEWAY` only when an explicit
gateway is configured. `workspace_mode = "upload"` stages the active workdir
into `/sandbox/<basename>` before Bash, which avoids macOS external-volume paths
such as `/Volumes/...` leaking into the sandbox. `remote` runs from
`remote_workdir` or `/sandbox`; `mirror` assumes the active workspace path is
visible inside the sandbox runtime.

`archon sandbox status --verbose` shows backend-specific safety knobs, including
Docker host-mount settings and OpenShell provider-injection/host-shell-fallback
flags. `archon sandbox explain --backend <name>` expands that into the
permission flow, mount/workspace policy, network policy, and credential-redaction
posture before any command is run. `archon sandbox doctor --backend <name>` is
also recorded as a redacted Cozo sandbox runtime event.

Interactive sessions wrap the selected sandbox backend with a Cozo audit layer.
Tool checks and backend Bash execution decisions are recorded as
`sandbox_runtime_events`, and each session creates a `sandbox_sessions` row with
redacted transport details. Commands, environment values, and workspace paths
are not stored in the audit payload. Denied or failed sandbox decisions also
write an agent performance ledger signal so governed evolution can see repeated
sandbox failures without learning around the isolation boundary.

## OpenShell Policy

OpenShell defaults are deliberately conservative:

```toml
provider_injection = false
host_shell_fallback = false
workspace_mode = "upload"
gateway = "openshell"
```

OpenShell execution must use the OpenShell transport or configured gateway. It
must not inject provider credentials into the sandbox, and it must not silently
execute directly on the host.

See also:

- [Sandbox real-world use cases](../cookbook/sandbox-real-world-use-cases.md)
- [Docker sandbox](docker-sandbox.md)
- [SSH sandbox](ssh-sandbox.md)
- [OpenShell sandbox](openshell-sandbox.md)
- [Tool preflight](tool-preflight.md)
