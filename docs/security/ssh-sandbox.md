# SSH Sandbox

The SSH backend routes Bash through a configured remote target when
`sandbox.backend = "ssh"` and `sandbox.ssh.enabled = true`.

## Configuration

```toml
[sandbox]
backend = "ssh"

[sandbox.ssh]
enabled = true
host = "sandbox.example"
user = "archon"
port = 22
workspace_mode = "remote"
remote_workdir = "/srv/archon/workspace"
host_key_checking = true
host_shell_fallback = false
```

Remote mode requires `remote_workdir`. Mirror mode assumes the current local
workspace path also exists on the remote host.

## Workspace Filesystem

In `remote` mode the file tools operate on `remote_workdir` over the same SSH
transport Bash uses, so `Read` returns what Bash would see. **This is a change:**
`Write` and `Edit` previously mutated the host while Bash ran remotely. Use
`mirror` mode if you want the file tools on the local tree.

Remote paths must be absolute — a relative path has no defined meaning on the
far side. Paths are single-quoted for the remote shell, and file contents are
carried base64-encoded in both directions, because the remote runs a login shell
and a profile that prints would otherwise corrupt a transfer undetectably. A
write goes to a sibling temp file and is renamed, and the remote echoes the
resulting byte count so a dropped transfer is an error rather than a truncated
file.

The remote target needs `base64` and, for `**` patterns, a Bash new enough for
`globstar`. Both are reported by name if missing rather than failing obscurely.

## Safety Posture

SSH execution uses strict host-key checking, batch mode, no agent forwarding, no
local command hooks, and no environment forwarding. Provider credentials, SSH
agents, Git credentials, generated memory databases, and arbitrary host paths
are not sent to the remote target.

If preflight fails, SSH returns an error and does not fall back to the host
shell.

## Commands

```bash
archon sandbox explain --backend ssh
archon sandbox doctor --backend ssh
```
