# Domain Context

## Background
Rust systems programming operates at the boundary between high-level safety guarantees and low-level hardware control. The ownership system, borrow checker, and lifetime annotations are the primary mechanisms for achieving memory safety without garbage collection.

## Key Concepts
- **Ownership**: Every value has exactly one owner. When the owner goes out of scope, the value is dropped.
- **Borrowing**: References (`&T`, `&mut T`) borrow values without taking ownership. At most one `&mut` OR any number of `&` at a time.
- **Lifetimes**: Compiler-tracked scopes ensuring references never outlive their referents. Named lifetimes (`'a`) express relationships.
- **Send/Sync**: `Send` = safe to transfer between threads. `Sync` = safe to share references between threads. Auto-traits derived from fields.
- **Pin**: Guarantees a value will not be moved in memory. Critical for self-referential structs and async state machines.
- **Zero-cost abstractions**: Abstractions that compile down to the same machine code as hand-written equivalents. Iterators, closures, and trait objects (when monomorphized) are canonical examples.
- **Soundness**: An API is sound if no sequence of safe calls can trigger undefined behavior. Unsound APIs are bugs.

## Common Patterns
- **Typestate pattern**: Encode state transitions in the type system so invalid transitions are compile errors
- **Newtype pattern**: Wrap primitives in single-field structs for type safety (`struct UserId(u64)`)
- **Builder pattern**: Fluent API for constructing complex objects with compile-time required field enforcement
- **RAII guards**: Implement `Drop` for cleanup -- `MutexGuard`, `File`, custom resource handles
- **Interior mutability**: `Cell<T>`, `RefCell<T>`, `Mutex<T>`, `RwLock<T>`, atomics for shared mutation
- **Error enums**: One enum per module/crate with variants for each failure mode, implementing `std::error::Error`
