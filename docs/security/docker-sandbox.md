# Docker Sandbox

The Docker backend provides local process isolation for Bash when
`sandbox.backend = "docker"` and `sandbox.docker.enabled = true`.

Install host Docker dependencies with:

```bash
sudo scripts/install-system-deps.sh --with-docker
archon sandbox doctor --backend docker
```

## Default Posture

Docker runs with:

- `--cap-drop ALL`
- `--security-opt no-new-privileges`
- no Docker socket mount
- no broad home mount
- no privileged mode
- configured CPU and memory limits
- network disabled or explicitly configured

The workspace is read-only by default. `workspace_access = "rw"` mounts it
read-write. `workspace_access = "scratch"` keeps the workspace read-only and
adds ephemeral `/scratch`.

The default `sandbox.mode = "risky"` routes Bash through Docker while leaving
normal host-side coding tools under permission preflight. Use
`sandbox.mode = "all"` only for strict sessions where unsupported host-side
tools should be blocked.

## Workspace Paths

The workspace is bind-mounted, so the container and the host hold the same
bytes. What differs is the path: Bash runs with `--workdir /workspace`, so
everything it prints — a compiler error, `find` output, a stack trace — names
paths under `/workspace`.

Those paths resolve. Handing `/workspace/src/main.rs` to `Read` reads the same
file the container just wrote, and `Glob` returns container paths so they can be
pasted straight into a Bash command. Host paths keep working unchanged.

Two paths are refused rather than resolved:

- A `/workspace/...` path containing `..` that would climb out of the mount.
  Escaping the mount is the one thing the mount exists to prevent.
- Anything under `/scratch`. That tmpfs exists only inside the container and is
  discarded with it, so it has no host file at all. The error says so rather
  than reporting a missing file.

## Writable Paths

Use relative `docker.writable_paths` when a mostly read-only workspace needs a
specific writable subpath. Absolute paths, parent traversal, commas, and NUL are
rejected.

## Commands

```bash
archon sandbox status --verbose
archon sandbox explain --backend docker
archon sandbox doctor --backend docker
```

Doctor is detect-only. Actual Bash routing happens through the runtime sandbox
backend during tool execution.

For scenario-driven setup examples, see
[Sandbox real-world use cases](../cookbook/sandbox-real-world-use-cases.md).
