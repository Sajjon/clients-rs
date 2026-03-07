# How `dep` Works

This crate has two layers:

- the runtime in [`src/lib.rs`](./src/lib.rs)
- the proc-macro expansion logic in [`dep-macros/src/lib.rs`](./dep-macros/src/lib.rs)

The design goal is simple: let users write dependency clients as plain structs
with methods, while keeping overrides lightweight and avoiding trait objects.

## The mental model

Every dependency client in `dep` is a concrete value type.

When you write:

```rust
use dep::client;

client! {
    pub struct Clock as clock {
        pub fn now_millis() -> u64 = || 1234;
    }
}
```

the generated `Clock` is conceptually close to:

```rust
#[derive(Clone, Copy)]
pub struct Clock {
    now_millis: fn() -> u64,
}

impl Clock {
    pub fn now_millis(&self) -> u64 {
        (self.now_millis)()
    }
}
```

That is the core trick of the crate:

- methods are stored as raw function pointers
- the client stays a plain concrete struct
- cloning a client is cheap
- overriding a method means replacing a function pointer

## What `client!` generates

`client!` expands more than just the struct and its methods.

For each client, the macro also generates:

- `impl dep::Dependency for Client`, so `get::<Client>()` can resolve a live value
- `impl Default for Client`, forwarding to `Dependency::live()`
- a helper module named after the `as ...` clause
- one nested helper module per method

For a declaration like:

```rust
client! {
    pub struct Clock as clock {
        pub fn now_millis() -> u64 = || 1234;
    }
}
```

the helper shape is roughly:

```rust
pub mod clock {
    pub fn get() -> super::Clock { ... }

    pub mod now_millis {
        pub fn get() -> fn() -> u64 { ... }
        pub fn override_with<F>(builder: &mut dep::OverrideBuilder, implementation: F)
        where
            F: Fn() -> u64 + Copy + 'static,
        { ... }
    }
}
```

Those helper modules are what power:

- `deps!`, which calls `clock::now_millis::get()`
- `test_deps!`, which calls `clock::now_millis::override_with(...)`

This is why user code can stay flat and readable without manually dealing with
whole client values in the common case.

## Live implementations and missing implementations

Each declared method has two possible forms:

1. with a live implementation
2. without one

With an implementation:

```rust
pub fn now_millis() -> u64 = || 1234;
```

the macro stores an erased function pointer in the generated `Clock::live()`.

Without an implementation:

```rust
pub fn fetch_user(id: u64) -> Result<User, UserClientError>;
```

the macro generates a default function pointer that panics via
`dep::unimplemented_dependency("UserClient.fetch_user")`.

That keeps the client usable in tests even when the live dependency is supposed
to be supplied only by test overrides.

## How overrides are stored

The runtime stores overrides in a process-global stack:

```rust
RwLock<Vec<HashMap<TypeId, Box<dyn Any + Send + Sync>>>>
```

The important properties are:

- overrides are keyed by concrete dependency type
- newer layers shadow older layers
- dropping a guard pops exactly one layer

`OverrideBuilder::enter()` pushes a layer and returns an `OverrideGuard`.
Dropping the guard removes that layer.

`OverrideBuilder::enter_test()` does the same thing, but also acquires a
process-wide spin lock so parallel tests do not trample each other.

When you call `get::<D>()`, the runtime:

1. looks through override layers from newest to oldest
2. clones the first matching dependency value if one exists
3. otherwise falls back to `D::live()`

That means dependency resolution is global and dynamic, but only at the level
of whole clients. Once you have a client value, calling one of its methods is
just a function-pointer call.

## Why `define_erasers!` exists

The most unusual part of the crate is the closure erasure machinery in
`define_erasers!` inside [`src/lib.rs`](./src/lib.rs).

Rust lets non-capturing closures coerce to function pointers, but this crate
needs a uniform, reusable way to do that inside generated code for multiple
arities and for both sync and async methods.

The runtime therefore generates a family of helpers like:

- `erase_sync_0`
- `erase_sync_1`
- `erase_sync_2`
- `erase_async_0`
- `erase_async_1`
- ...

Each eraser:

1. accepts a closure type `F`
2. checks that `F` is non-capturing with `assert_non_capturing::<F>()`
3. builds a monomorphized trampoline function
4. returns that trampoline as a plain `fn(...) -> ...`

Conceptually, a sync eraser looks like this:

```rust
pub fn erase_sync_1<F, R, A0>(_: F) -> fn(A0) -> R
where
    F: Fn(A0) -> R + Copy + 'static,
{
    assert_non_capturing::<F>();

    fn trampoline<F, R, A0>(arg0: A0) -> R
    where
        F: Fn(A0) -> R + Copy + 'static,
    {
        let function: F = unsafe { resurrect_zst() };
        function(arg0)
    }

    trampoline::<F, R, A0>
}
```

The async version does the same thing, except it boxes the returned future into
`dep::BoxFuture<R>`.

## Why the erasers need `unsafe`

The trampoline is just a plain function pointer. It cannot capture the original
closure value. So the runtime needs some way to recreate the closure inside the
trampoline body.

That is what `resurrect_zst::<F>()` does.

This is only valid because `dep` enforces a strong invariant first:

- the closure must be non-capturing
- non-capturing closures and function items are zero-sized
- zero-sized closure types carry no runtime state to preserve

So the flow is:

1. reject any capturing closure by checking `size_of::<F>() == 0`
2. inside the trampoline, reconstruct the zero-sized value of `F`
3. invoke it immediately

That is the only runtime `unsafe` that the crate relies on for normal
operation. Everything else in the dependency lookup and override machinery is
ordinary safe Rust.

## Async methods

Async methods need one extra step.

A raw function pointer cannot directly return an opaque future type because each
closure would otherwise produce a different anonymous future type. To keep the
generated client concrete, async methods are stored as:

```rust
fn(...) -> dep::BoxFuture<R>
```

So the async eraser:

- reconstructs the non-capturing closure
- calls it to get the concrete future
- boxes that future

That is why async dependency methods allocate once per call.

## How `deps!` stays small

`deps!` does not do any special runtime work. It is just syntax sugar.

```rust
deps! {
    now = clock.now_millis,
}
```

expands roughly to:

```rust
let now = clock::now_millis::get();
```

That local `now` binding is already a raw function pointer, so calling `now()`
after that is cheap.

## How `test_deps!` stays flat

`test_deps!` is also mostly syntax sugar around `OverrideBuilder`.

```rust
test_deps! {
    clock.now_millis => || 1234,
}
```

expands roughly to:

1. create an `OverrideBuilder`
2. ask `clock::now_millis::override_with(...)` to replace that one field
3. call `enter_test()`
4. bind the resulting guard to a hidden local variable

The hidden guard lives for the rest of the current scope, which is why tests
can stay unindented.

## How `#[derive(Depends)]` works

`#[derive(Depends)]` is intentionally simple.

For each field:

- `#[dep] field: T` becomes `field: dep::get::<T>()`
- every other field becomes `field: Default::default()`

The derive then generates:

- `impl Default`
- `fn from_deps() -> Self`

It does not currently support generics or where-clauses. That is a limitation
of the current parser, not of the overall runtime model.

## Performance summary

In practice the cost model is:

- one override lookup when resolving a client
- one function-pointer call per sync dependency method invocation
- one boxed future allocation per async dependency method invocation

That makes `dep` a good fit for application-level dependency wiring and tests.
It is less well suited to designs that expect repeated global resolution inside
tight inner loops.
