# dep

> [!NOTE]
> This is a POC fully implemented by ChatGTP 5.4 with
> steering from Alex Cyon looking for a simpler DI 
> solution for Rust - inspired by [`swift-dependencies`](https://github.com/pointfreeco/swift-dependencies)

`dep` is a concrete-struct dependency injection library for Rust.

The core idea is simple:

- dependencies are plain structs, not traits
- each dependency method is backed by a raw function pointer
- production code resolves concrete clients from `Dependency::live()`
- tests override individual methods with almost no ceremony

This is intentionally closer to `swift-dependencies` than to traditional Rust DI crates built around `Arc<dyn Trait>` or `Box<dyn Trait>`.

## Why this crate exists

Many Rust DI solutions lean on trait objects. That works, but it often adds boilerplate:

- defining traits just for testability
- threading `Arc<dyn Client>` or `Box<dyn Client>` through the app
- writing mock structs or mock frameworks

`dep` takes a different route. A dependency client is a concrete `struct` whose fields are function pointers. Methods call those function pointers. Tests swap pointers in scoped override layers.

The result is:

- concrete dependency types
- direct call sites
- fast, flat tests
- support for sync and async APIs

## Quick start

```rust
use dep::{DependencyError, client, deps, test_deps};

#[derive(Debug, Clone, PartialEq, Eq)]
struct User {
    id: u64,
    name: String,
}

client! {
    pub struct UserClient as user_client {
        pub fn fetch_user(id: u64) -> Result<User, DependencyError> = |_id| {
            Err(DependencyError::missing("user_client.fetch_user"))
        };
    }
}

client! {
    pub struct Clock as clock {
        pub fn now_millis() -> u64 = || 0;
    }
}

pub fn greeting_for_user(id: u64) -> Result<String, DependencyError> {
    deps! {
        fetch_user = user_client.fetch_user,
        now = clock.now_millis,
    }

    let user = fetch_user(id)?;
    let now = now();
    Ok(format!("Hello, {} @ {}", user.name, now))
}

#[test]
fn greeting_uses_flat_test_overrides() {
    test_deps! {
        user_client.fetch_user => |id| Ok(User { id, name: "Blob".into() }),
        clock.now_millis => || 1234,
    }

    let result = greeting_for_user(42).unwrap();
    assert_eq!(result, "Hello, Blob @ 1234");
}
```

## Defining clients

Use the `client!` proc macro to declare a dependency client:

```rust
use dep::{DependencyError, client};

#[derive(Debug, Clone, PartialEq, Eq)]
struct User {
    id: u64,
    name: String,
}

client! {
    pub struct UserClient as user_client {
        pub fn fetch_user(id: u64) -> Result<User, DependencyError>;
    }
}
```

This generates:

- a concrete `UserClient` struct
- methods like `user_client.fetch_user`
- a `Dependency` implementation that resolves the live client
- a helper module named `user_client`
- nested helper modules like `user_client::fetch_user`

If you omit the implementation on a method, calling it without a test override panics with a descriptive error. That is useful when you want tests to provide the implementation explicitly.

## Using dependencies in global functions

The `deps!` macro binds dependency methods to local names:

```rust
use dep::{DependencyError, client, deps};

client! {
    struct Clock as clock {
        fn now_millis() -> u64 = || 1234;
    }
}

fn now_string() -> Result<String, DependencyError> {
    deps! {
        now = clock.now_millis,
    }

    Ok(now().to_string())
}

assert_eq!(now_string().unwrap(), "1234");
```

This is especially useful in free functions where you do not want to thread a container or context object through the call stack.

## Using dependencies as fields

Use `#[derive(Depends)]` plus `#[dep]` to build structs from dependencies:

```rust
use dep::{DependencyError, Depends, client};

#[derive(Debug, Clone, PartialEq, Eq)]
struct User {
    id: u64,
    name: String,
}

client! {
    struct UserClient as user_client {
        fn fetch_user(id: u64) -> Result<User, DependencyError> = |id| {
            Ok(User { id, name: format!("User {id}") })
        };
    }
}

client! {
    struct Clock as clock {
        fn now_millis() -> u64 = || 1234;
    }
}

#[derive(Depends)]
struct Greeter {
    #[dep]
    user_client: UserClient,
    #[dep]
    clock: Clock,
}

impl Greeter {
    fn greeting_for_user(&self, id: u64) -> Result<String, DependencyError> {
        let user = self.user_client.fetch_user(id)?;
        Ok(format!("Hello, {} @ {}", user.name, self.clock.now_millis()))
    }
}

let greeter = Greeter::from_deps();
assert_eq!(greeter.greeting_for_user(7).unwrap(), "Hello, User 7 @ 1234");
```

`Depends` currently generates:

- `Default`, where `#[dep]` fields resolve from the dependency system and all other fields use `Default::default()`
- `from_deps()`, a convenience constructor that forwards to `Default`

## Async support

Async dependency methods are supported directly:

```rust
use dep::{DependencyError, client, deps, test_deps};

client! {
    struct AsyncClock as async_clock {
        async fn now_millis() -> u64 = || async { 4321 };
    }
}

async fn read_now() -> Result<u64, DependencyError> {
    deps! {
        now = async_clock.now_millis,
    }

    Ok(now().await)
}

test_deps! {
    async_clock.now_millis => || async { 9999 },
}

# fn block_on<F: std::future::Future>(future: F) -> F::Output {
#     use std::pin::Pin;
#     use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
#     let mut future = Box::pin(future);
#     unsafe fn clone(_: *const ()) -> RawWaker { RawWaker::new(std::ptr::null(), &VTABLE) }
#     unsafe fn wake(_: *const ()) {}
#     unsafe fn wake_by_ref(_: *const ()) {}
#     unsafe fn drop(_: *const ()) {}
#     static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);
#     let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
#     let mut context = Context::from_waker(&waker);
#     loop {
#         match Pin::as_mut(&mut future).poll(&mut context) {
#             Poll::Ready(value) => break value,
#             Poll::Pending => std::thread::yield_now(),
#         }
#     }
# }
assert_eq!(block_on(read_now()).unwrap(), 9999);
```

## Client-on-client composition

Clients can depend on other clients directly:

```rust
use dep::{DependencyError, client, deps};

client! {
    struct Clock as clock {
        fn now_millis() -> u64 = || 1234;
    }
}

client! {
    struct Formatter as formatter {
        fn formatted_now() -> Result<String, DependencyError> = || {
            deps! {
                now = clock.now_millis,
            }

            Ok(format!("now={}", now()))
        };
    }
}

assert_eq!(formatter::get().formatted_now().unwrap(), "now=1234");
```

## Fine-grained override control

Most tests should use `test_deps!`, but there is also a lower-level API:

```rust
use dep::{OverrideBuilder, client, erase_sync_0, get};

client! {
    struct Clock as clock {
        fn now_millis() -> u64 = || 0;
    }
}

let _test_scope = OverrideBuilder::new().enter_test();

let override_clock = Clock {
    now_millis: erase_sync_0(|| 1234),
};

let _guard = OverrideBuilder::new().set(override_clock).enter();

assert_eq!(get::<Clock>().now_millis(), 1234);
```

This is useful when you want to replace a whole client at once or compute an override from the current client via `OverrideBuilder::update`.

## Supported today

- sync dependency methods
- async dependency methods
- dependency access inside free functions via `deps!`
- dependency access inside structs via `#[derive(Depends)]`
- nested dependency scopes
- low-ceremony test overrides via `test_deps!`
- dependencies implemented in terms of other dependencies
- direct builder-based override control through `OverrideBuilder`

## Current limitations

- live implementations and test overrides must be non-capturing closures or function items
- `client!` currently supports up to 4 method arguments
- `Depends` currently supports simple braced structs and does not handle generics or where-clauses
- override state is process-global rather than task-local
- `test_deps!` serializes scopes within a process so concurrent tests do not trample each other

## Relationship to Rust trait-based DI

`dep` does not replace all trait-based design. Trait objects are still a good fit when you genuinely need polymorphism as part of the domain model.

This crate is for a narrower problem:

- you want ergonomic dependency injection
- you want very light test setup
- you prefer concrete clients
- you do not want to define traits solely for testability

That trade-off is the entire point of the crate.
