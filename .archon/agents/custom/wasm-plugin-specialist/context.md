# Domain Context

## Background
WebAssembly plugin systems enable running untrusted or third-party code safely within a host application. The host provides a controlled API surface; the guest (plugin) runs in a sandboxed linear memory space with no direct access to host resources unless explicitly granted. The WASM Component Model and WASI are converging to standardize this pattern.

## Key Concepts
- **Linear Memory**: WASM modules operate on a flat byte array. All data passes through this memory. The host and guest must agree on memory layout for complex types.
- **Component Model**: Higher-level abstraction over core WASM. Defines interfaces via WIT (WASM Interface Types), enabling rich type exchange without manual serialization.
- **WIT (WASM Interface Types)**: IDL for defining WASM component interfaces. Supports records, variants, enums, flags, resources, lists, options, results, and futures.
- **WASI (WebAssembly System Interface)**: Standardized system-level APIs (filesystem, sockets, clocks, random) with capability-based security. Preview 2 uses the Component Model.
- **Fuel Metering**: Runtime mechanism to limit WASM execution by consuming "fuel" per instruction. Prevents infinite loops and resource exhaustion.
- **Epoch Interruption**: Timer-based interruption where the host increments an epoch counter and WASM execution yields at checkpoints. Lower overhead than fuel.
- **Canonical ABI**: The Component Model's binary encoding for passing values across the host-guest boundary. Handles strings (UTF-8/UTF-16), lists, records, variants.
- **Instance Pooling**: Pre-allocating WASM instance slots for fast instantiation. Critical for serverless/request-per-plugin architectures.

## Common Patterns
- **Capability handles**: Pass opaque resource handles to plugins instead of raw access -- plugin calls host functions with the handle to perform operations
- **Event-driven plugins**: Host calls well-known exported functions (`on_request`, `on_event`, `init`, `shutdown`) -- plugin registers interest
- **Plugin discovery**: Scan a directory for `.wasm` files, validate WIT world compatibility, instantiate on demand
- **Hot reload**: Watch plugin directory, recompile/reinstantiate on change without restarting the host
- **Version negotiation**: Plugin exports a `version()` function or the host checks WIT world compatibility before instantiation
