use clients::{DependencyError, Depends, OverrideBuilder, client, deps, get, test_deps};
use std::any::Any;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

#[derive(Debug, Clone, PartialEq, Eq)]
struct User {
    id: u64,
    name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserClientError {
    Unavailable,
}

client! {
    pub struct UserClient as user_client {
        pub fn fetch_user(id: u64) -> Result<User, UserClientError>;
    }
}

client! {
    pub struct Clock as clock {
        pub fn now_millis() -> u64 = || 0;
    }
}

client! {
    pub struct GreetingClient as greeting_client {
        pub fn greeting_for_user(id: u64) -> Result<String, UserClientError> = |id| {
            deps! {
                fetch_user = user_client.fetch_user,
                now = clock.now_millis,
            }

            let user = fetch_user(id)?;
            Ok(format!("Hello, {} @ {}", user.name, now()))
        };
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsyncUserClientError {
    Unavailable,
}

client! {
    pub struct AsyncUserClient as async_user_client {
        pub async fn fetch_user(id: u64) -> Result<User, AsyncUserClientError>;
    }
}

client! {
    pub struct AsyncClock as async_clock {
        pub async fn now_millis() -> u64 = || async { 0 };
    }
}

client! {
    pub struct GenericClient as generic_client {
        pub fn summarize(inputs: Vec<Result<u64, DependencyError>>) -> usize = |inputs: Vec<Result<u64, DependencyError>>| {
            inputs.len()
        };
    }
}

client! {
    pub struct CancelClient as cancel_client {
        pub fn is_cancelled(cancel_flag: Option<&Arc<AtomicBool>>) -> bool = |cancel_flag: Option<&Arc<AtomicBool>>| {
            cancel_flag.is_some_and(|flag| flag.load(Ordering::Relaxed))
        };
    }
}

client! {
    pub struct AsyncCancelClient as async_cancel_client {
        pub async fn is_cancelled(cancel_flag: Option<&Arc<AtomicBool>>) -> bool = |cancel_flag: Option<&Arc<AtomicBool>>| {
            let was_cancelled = cancel_flag.is_some_and(|flag| flag.load(Ordering::Relaxed));
            async move { was_cancelled }
        };
    }
}

mod external_client_crate {
    use clients::client;

    client! {
        pub struct ExportedFooClient as foo_client {
            pub fn foo() -> String = || "from-exported-client".to_string();
        }
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
    pub fn greeting_for_user(&self, id: u64) -> Result<String, UserClientError> {
        let user = self.user_client.fetch_user(id)?;
        let now = self.clock.now_millis();
        Ok(format!("Hello, {} @ {}", user.name, now))
    }
}

#[derive(Depends)]
pub struct DerivedGenericHolder {
    #[dep]
    clock: Clock,
    cached_ids: Vec<Result<u64, DependencyError>>,
}

fn greeting_for_user(id: u64) -> Result<String, UserClientError> {
    deps! {
        fetch_user = user_client.fetch_user,
        now = clock.now_millis,
    }

    let user = fetch_user(id)?;
    let now = now();
    Ok(format!("Hello, {} @ {}", user.name, now))
}

async fn async_greeting_for_user(id: u64) -> Result<String, AsyncUserClientError> {
    deps! {
        fetch_user = async_user_client.fetch_user,
        now = async_clock.now_millis,
    }

    let user = fetch_user(id).await?;
    let now = now().await;
    Ok(format!("Hello, {} @ {}", user.name, now))
}

fn panic_message(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => (*message).to_string(),
            Err(_) => "<non-string panic payload>".into(),
        },
    }
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

    let greeter = Greeter::from_deps();
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
fn omitted_client_implementation_panics_with_actionable_diagnostics() {
    let _test_scope = OverrideBuilder::new().enter_test();
    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = get::<UserClient>().fetch_user(42);
    }))
    .expect_err("missing client implementation should panic");

    let message = panic_message(panic);
    assert!(message.contains("UserClient.fetch_user"));
    assert!(message.contains("test_deps!"));
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

#[test]
fn client_macro_handles_generic_argument_types() {
    let client = get::<GenericClient>();
    let result = client.summarize(vec![Ok(1), Err(DependencyError::message("boom"))]);
    assert_eq!(result, 2);
}

#[test]
fn depends_derive_handles_generic_non_dependency_fields() {
    test_deps! {
        clock.now_millis => || 2468,
    }

    let holder = DerivedGenericHolder::from_deps();
    assert_eq!(holder.clock.now_millis(), 2468);
    assert!(holder.cached_ids.is_empty());
}

#[test]
fn client_macro_supports_borrowed_arguments_in_live_sync_implementations() {
    let cancelled = Arc::new(AtomicBool::new(true));

    assert!(get::<CancelClient>().is_cancelled(Some(&cancelled)));
    assert!(!get::<CancelClient>().is_cancelled(None));
}

#[test]
fn test_deps_supports_borrowed_arguments_in_sync_overrides() {
    test_deps! {
        cancel_client.is_cancelled => |cancel_flag: Option<&Arc<AtomicBool>>| cancel_flag.is_some(),
    }

    let cancelled = Arc::new(AtomicBool::new(false));

    assert!(get::<CancelClient>().is_cancelled(Some(&cancelled)));
    assert!(!get::<CancelClient>().is_cancelled(None));
}

#[test]
fn client_macro_supports_borrowed_arguments_in_live_async_implementations() {
    let cancelled = Arc::new(AtomicBool::new(true));

    assert!(block_on(
        get::<AsyncCancelClient>().is_cancelled(Some(&cancelled))
    ));
    assert!(!block_on(get::<AsyncCancelClient>().is_cancelled(None)));
}

#[test]
fn test_deps_supports_borrowed_arguments_in_async_overrides() {
    test_deps! {
        async_cancel_client.is_cancelled => |cancel_flag: Option<&Arc<AtomicBool>>| {
            let was_cancelled = cancel_flag.is_some();
            async move { was_cancelled }
        },
    }

    let cancelled = Arc::new(AtomicBool::new(false));

    assert!(block_on(
        get::<AsyncCancelClient>().is_cancelled(Some(&cancelled))
    ));
    assert!(!block_on(get::<AsyncCancelClient>().is_cancelled(None)));
}

#[test]
fn deps_macro_supports_qualified_client_paths() {
    deps! {
        foo = external_client_crate::foo_client.foo,
    }

    assert_eq!(foo(), "from-exported-client");
}

#[test]
fn test_deps_supports_qualified_client_paths() {
    test_deps! {
        external_client_crate::foo_client.foo => || "overridden-from-qualified-path".to_string(),
    }

    deps! {
        foo = external_client_crate::foo_client.foo,
    }

    assert_eq!(foo(), "overridden-from-qualified-path");
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

// ---------------------------------------------------------------------------
// #[arg] attribute tests
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
enum LogLevel {
    Info,
    Debug,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Config {
    log_level: LogLevel,
    base_url: String,
}

#[derive(Depends)]
struct ServiceWithArg {
    #[dep]
    clock: Clock,
    #[arg]
    config: Config,
}

#[test]
fn arg_field_is_passed_through_from_deps() {
    test_deps! {
        clock.now_millis => || 7777,
    }

    let config = Config {
        log_level: LogLevel::Debug,
        base_url: "https://test.example.com".into(),
    };
    let service = ServiceWithArg::from_deps(config.clone());

    assert_eq!(service.clock.now_millis(), 7777);
    assert_eq!(service.config, config);
}

#[derive(Depends)]
struct LayerA {
    #[dep]
    user_client: UserClient,
}

impl LayerA {
    fn fetch(&self, id: u64) -> Result<User, UserClientError> {
        self.user_client.fetch_user(id)
    }
}

#[derive(Depends)]
struct LayerB {
    layer_a: LayerA,
}

#[derive(Depends)]
struct LayerC {
    layer_b: LayerB,
    #[arg]
    tag: String,
}

#[test]
fn deep_nesting_resolves_deps_through_default_chain() {
    test_deps! {
        user_client.fetch_user => |id| Ok(User { id, name: "Deep".into() }),
    }

    let app = LayerC::from_deps("my-tag".to_string());

    assert_eq!(app.tag, "my-tag");
    assert_eq!(
        app.layer_b.layer_a.fetch(1).unwrap(),
        User {
            id: 1,
            name: "Deep".into()
        }
    );
}
