Vendored portable-pty 0.9.0 (MIT), from the locked crates.io source.

The Unix spawn hook uses archon-shell's syscall-only descriptor sweep instead
of allocating through /dev/fd enumeration after fork. The command builder is
split at existing impl boundaries to keep source files below 500 lines.
Other behavior and platform backends are unchanged.

Examples and their unused futures/smol dev dependencies are omitted.
