use dep::{DependencyError, Depends, client, deps, get, test_deps};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

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

client! {
    pub struct GreetingClient as greeting_client {
        pub fn greeting_for_user(id: u64) -> Result<String, DependencyError> = |id| {
            deps! {
                fetch_user = user_client.fetch_user,
                now = clock.now_millis,
            }

            let user = fetch_user(id)?;
            Ok(format!("Hello, {} @ {}", user.name, now()))
        };
    }
}

client! {
    pub struct AsyncUserClient as async_user_client {
        pub async fn fetch_user(id: u64) -> Result<User, DependencyError> = |_id| async move {
            Err(DependencyError::missing("async_user_client.fetch_user"))
        };
    }
}

client! {
    pub struct AsyncClock as async_clock {
        pub async fn now_millis() -> u64 = || async { 0 };
    }
}

#[derive(Depends)]
pub struct Greeter {
    #[dep]
    user_client: UserClient,
    #[dep]
    clock: Clock,
}

impl Greeter {
    pub fn greeting_for_user(&self, id: u64) -> Result<String, DependencyError> {
        let user = self.user_client.fetch_user(id)?;
        let now = self.clock.now_millis();
        Ok(format!("Hello, {} @ {}", user.name, now))
    }
}

fn greeting_for_user(id: u64) -> Result<String, DependencyError> {
    deps! {
        fetch_user = user_client.fetch_user,
        now = clock.now_millis,
    }

    let user = fetch_user(id)?;
    let now = now();
    Ok(format!("Hello, {} @ {}", user.name, now))
}

async fn async_greeting_for_user(id: u64) -> Result<String, DependencyError> {
    deps! {
        fetch_user = async_user_client.fetch_user,
        now = async_clock.now_millis,
    }

    let user = fetch_user(id).await?;
    let now = now().await;
    Ok(format!("Hello, {} @ {}", user.name, now))
}

#[test]
fn overrides_work_in_global_functions() {
    test_deps! {
        user_client.fetch_user => |id| Ok(User { id, name: "Blob".into() }),
        clock.now_millis => || 1234,
    }

    let result = greeting_for_user(42).unwrap();
    assert_eq!(result, "Hello, Blob @ 1234");
}

#[test]
fn derives_dependencies_into_fields() {
    test_deps! {
        user_client.fetch_user => |id| Ok(User { id, name: "Blob".into() }),
        clock.now_millis => || 5678,
    }

    let greeter = Greeter::default();
    let result = greeter.greeting_for_user(7).unwrap();
    assert_eq!(result, "Hello, Blob @ 5678");
}

#[test]
fn clients_can_depend_on_clients() {
    test_deps! {
        user_client.fetch_user => |id| Ok(User { id, name: "Blob".into() }),
        clock.now_millis => || 999,
    }

    let client = get::<GreetingClient>();
    let result = client.greeting_for_user(1).unwrap();
    assert_eq!(result, "Hello, Blob @ 999");
}

#[test]
fn async_dependencies_work() {
    test_deps! {
        async_user_client.fetch_user => |id| async move {
            Ok(User { id, name: "Async Blob".into() })
        },
        async_clock.now_millis => || async { 4321 },
    }

    let result = block_on(async_greeting_for_user(5)).unwrap();
    assert_eq!(result, "Hello, Async Blob @ 4321");
}

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    let mut future = Box::pin(future);
    let waker = noop_waker();
    let mut context = Context::from_waker(&waker);

    loop {
        match Pin::as_mut(&mut future).poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

fn noop_waker() -> Waker {
    unsafe fn clone(_: *const ()) -> RawWaker {
        RawWaker::new(std::ptr::null(), &VTABLE)
    }

    unsafe fn wake(_: *const ()) {}
    unsafe fn wake_by_ref(_: *const ()) {}
    unsafe fn drop(_: *const ()) {}

    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);

    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}
