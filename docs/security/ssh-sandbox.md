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
