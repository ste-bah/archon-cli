Vendored portable-pty 0.9.0 (MIT), from the locked crates.io source.

Archon changes:

- The Unix spawn hook uses archon-shell's syscall-only descriptor sweep instead
  of allocating through /dev/fd enumeration after fork.
- Unix commands are built with `archon_shell::spawn::command`, and the
  builder's environment is applied with `archon_shell::spawn::replace_environment`,
  so jobserver flags that name inherited descriptors are removed after the
  overlay (the child inherits only stdio).
- The command builder sanitizes jobserver flags when an environment value is
  set and when the base environment is captured.
- The command builder is split at existing impl boundaries to keep source
  files below 500 lines.

Other behavior and the Windows backend are unchanged.

Examples and their unused futures/smol dev dependencies are omitted.
