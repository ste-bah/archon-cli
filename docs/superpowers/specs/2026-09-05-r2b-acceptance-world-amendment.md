# R2b Acceptance World Amendment

**Status:** APPROVED WITH CONDITIONS for the first slice on 2026-09-05; see controlling scope below.
**Date:** 2026-09-05. Source baseline: `0c81c4321`.
**Amends:** `2026-08-27-decomposition-r2-engine-native-design.md`, R2b ordering
and “Acceptance check execution boundary and R2a deferral” (line 711 at baseline).
**Does not amend:** R2a historical evidence, ObserveOnly authority, legacy
admission, symbolic host capabilities, no host execution of model command text,
no protected-source mutation, or separate approval for external implementation.

## Controlling first-slice conditions

The operator approved inline execution of 9.0 → 9 → 10 → 12A. First run one bounded
pinned Linux build; failure stops the slice. No endpoint allow rules or broker
forwarding ship initially: use an internal logging blackhole plus trustworthy
observation of attempts that cannot reach it, including guest loopback. Any attempt
is PolicyDenied even if a command returns zero; incomplete observation fails closed.
The endpoint allowance design below is retained for later separate approval only.

A run-scoped immutable build cache is produced by a trusted build of identical
source/toolchain inputs and cloned into each check's private target. Never reuse
mutable check outputs as trusted seeds. Record cache identity and verify isolation.
Tasks 1–8 and 11 require separate approval; no external implementation is authorized.

## Reason

An acceptance world must run build/test commands and temporary data mutations,
not merely refuse them safely. The original specification's empty guest
environment, immutable guest working roots and universally disabled network do
not express how a build output, writable data copy or approved service probe can
exist. Host-owned scratch redirection and tightly scoped service capabilities
are required; granting host writes or unrestricted networking is not the remedy.

No claim is made that every accepted model check is executable or semantically
sufficient. Judge acceptance is not an execution-readiness proof. Unsupported
platform/toolchain, absent offline dependencies, unavailable input mapping and
blocked endpoint access are reported separately from a failed criterion.

## Replacement acceptance-world policy

Replace the blanket R2b world paragraph with this policy upon approval:

> R2b evaluates model-authored acceptance, nested verifier and residual fail-closed
> command text only in a mandatory ephemeral isolated world. Host source,
> project inputs and frozen task artifacts are exported as immutable snapshots;
> configured build/output/data paths refer only to fresh writable scratch copies,
> never writable live host mounts. Guest layout preserves host-configured relative
> paths, including an explicitly mapped build target. Guest environment is built
> from empty with a closed set of host-configured nonsecret toolchain/scratch and
> endpoint bindings. Network access is denied by default; explicitly configured
> read/probe endpoint rules may be reached only through a host-controlled enforcing
> broker. No unrestricted host/Internet network, host sockets/devices, raw provider
> credentials or administrative daemon endpoints are exposed. Inputs, layout,
> toolchain/image, policy, endpoint access, resources, outputs and whole-world
> teardown are recorded. Model output cannot widen policy. Containment cannot be
> proved, setup is inadequate or teardown fails: record operational failure;
> never execute on the host or claim the criterion passed.

### Build and data semantics

1. Roots and writable relative directories come from host configuration, captured
   and hashed with observer policy. The model supplies only frozen command bytes
   and the existing closed `TrustedCwd` choice.
2. Keep source/manifests/lockfiles/tasks/pins immutable. Build/cache/home/tmp and
   expressly declared mutable project directories are fresh per-check scratch.
   Any guest mountpoint/link is assembled in run-owned exports, never live roots.
3. Host binds `CARGO_TARGET_DIR` to scratch **and** the configured relative `target`
   path to the same output. The built binary invoked via `./target/release/...`
   must be the one that invocation built. No copied stale host binary substitutes.
4. Copy declared project data into scratch at its original relative location;
   changes are evidence only, never copied back. Preserve required readable
   project artifacts outside that writable set. Scratch quotas/resource budgets
   come from the operator's build profile and are tested with a real build.
5. Host supplies a digest-pinned compatible guest toolchain, Cargo home/cache seeds
   free of credentials, and offline dependency policy. Missing dependencies must
   not cause silent Internet access. No host HOME, provider config or socket export.
6. Project and repository may be different roots. Default keeps them separate.
   An explicit host-configured combined guest project view may provide repository
   source with declared project inputs so build commands and `--target .` preserve
   their intended semantics. Refuse conflicting files or implicit source selection;
   record the mapping. Never detect a command word and silently move its cwd.
7. Linux guest support must be proved for the declared build profile. macOS-only
   projects need an independently reviewed compatible world backend, not host
   execution or a false “working” Docker claim.

### Endpoint policy

1. Default rules are empty; offline worlds use no network. Broker-enabled worlds
   have no direct egress, even if a command changes proxy/environment variables.
2. Each rule names guest route, exact upstream scheme/host/port, permitted HTTP
   methods/path prefixes or strict RPC operations, and request/response/time limits.
   No wildcard host/port, unrestricted CONNECT, broad host-gateway or host network.
3. Guest loopback is not host loopback. Explicit guest-local forwarders and an
   owned exact-upstream bridge provide approved loopback services without changing
   the protected service. Forwarding destination is never supplied by the model.
4. An endpoint is not safe merely because it is local. Administrative methods,
   filesystem mutation, arbitrary URL-fetch/tool-execution APIs and unconstrained
   tunnels remain denied. The first slice permits only side-effect-free read/probe
   operations. If operation-level enforcement cannot be implemented, refuse the
   rule; do not fall back to an unrestricted TCP port grant.
5. Address resolution and redirects are controlled by the broker. Pin/validate
   resolved addresses; validate TLS identity; revalidate each redirect. Block
   metadata and unlisted routes. Guest does not get unrestricted DNS access.
6. Local service rules do not authorize external provider destinations. External
   reads require an explicitly approved exact-origin broker rule, or an approved
   local service that implements the read. Readiness must identify this requirement
   before execution. No external origin is inferred from command text.
7. Credentials, if an approved read service needs them, stay in the trusted broker
   and never enter guest env, input copies or persisted evidence. Allowed request
   access still permits sending selected guest data to that endpoint; the operator
   must authorize that data-flow boundary. Request/response logging is redacted and
   bounded. No provider/model/configuration change is implied.

### Result semantics

- Ordinary executed nonzero exit is a failed acceptance criterion, not inherently
  a containment violation. Read-only checks of absent files may fail normally.
- Setup error, denied resource/endpoint, quota exhaustion, output truncation or
  incomplete teardown is operational evidence, even when a command catches an
  error and returns zero or labels policy denial as provider unavailability.
- Pass requires normal zero exit, adequate declared execution environment, no
  recorded policy denial and verified teardown. This does not make the judge's
  semantic assessment infallible.
- Freeze/terminal identities remain immutable. The observer never changes the
  implementation terminal status, even when global gate mode is enforce.

## First-slice lifecycle and order

Execute plan tasks **9 → 10 → 12A** first: real isolated execution, all observer
routes and independent acceptance-slice verification. Existing R2a terminal
snapshot and post-terminal observer suffice; full publication CAS, executable
snapshots and arbitrary-crash observer recovery are not prerequisites.

The world still owns a narrow execution lock, bounded guardian, parent EOF
cleanup, container lifetime watchdog and recorded owned-world identity. Do not
start two observers or automatically replay an ambiguously interrupted service
check. These minimum containment duties ship with task 9, not deferred to task 5.
One inline coordinator runs tests/Cargo; no implementation subagents.

Then, under separate continuation approval, tasks **1–8 → 11 → 12B** deliver the
remaining approved hardening and predecessor adoption. Task 12A cannot claim those
unimplemented guarantees. Full R2b completion, external-task implementation and
R3/R4 promotion remain separate gates. TD-034 is left independently tracked.

## Approval and required proofs

Approval must cover the writable scratch mapping and brokered-service amendment,
not merely the physical task reordering. Before enabling the backend, prove:

- actual guest Cargo build then relative built-binary invocation succeeds;
- scratch data mutation is visible inside that check, not live host data;
- project/repository-separated and configured combined layouts resolve correctly;
- permitted local service succeeds without permitting adjacent ports or methods;
- redirects, DNS/address changes, CONNECT, env overrides and direct sockets do not
  bypass default-deny enforcement;
- policy-denial-induced “unavailable” output is not an acceptance pass;
- timeout, parent death, fork/detach and output flood leave no owned world running;
- all three observer dispatch routes use this backend and remain ObserveOnly.

These are execution gates, not results obtained in this planning session. Frozen
real checks remain unchanged and unexecuted until separately authorized.
