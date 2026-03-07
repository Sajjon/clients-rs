use clients::{Clock, Env, Filesystem, Random, get};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::time::{Duration, SystemTime};

#[cfg(feature = "reqwest")]
use clients::http_client;
#[cfg(feature = "reqwest")]
use clients::test_deps;

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

#[test]
fn clock_exposes_now_and_sleep() {
    let _ = get::<Clock>().now();
    block_on(get::<Clock>().sleep(Duration::ZERO));
}

#[test]
fn env_exposes_process_information() {
    let env = get::<Env>();

    assert_eq!(
        env.current_dir().expect("current dir should resolve"),
        std::env::current_dir().expect("std current dir should resolve")
    );
    assert!(env.temp_dir().is_absolute());
    assert_eq!(env.var("__DEP_BUILTINS_SHOULD_NOT_EXIST__".into()), None);
}

#[test]
fn random_generates_bytes_and_integers() {
    let random = get::<Random>();

    let _ = random.next_u64();
    assert_eq!(random.fill_bytes(16).len(), 16);
}

#[test]
fn filesystem_reads_and_writes_bytes_and_strings() {
    let filesystem = get::<Filesystem>();
    let path = unique_temp_path("filesystem");

    filesystem
        .write_string(path.clone(), "hello".to_string())
        .expect("string write should succeed");
    assert_eq!(
        filesystem
            .read_string(path.clone())
            .expect("string read should succeed"),
        "hello"
    );

    filesystem
        .write(path.clone(), b"bytes".to_vec())
        .expect("binary write should succeed");
    assert_eq!(
        filesystem
            .read(path.clone())
            .expect("binary read should succeed"),
        b"bytes".to_vec()
    );

    std::fs::remove_file(path).expect("temp file should be removable");
}

#[cfg(feature = "uuid")]
#[test]
fn uuid_generates_uuid_strings() {
    let uuid = get::<clients::Uuid>().generate();
    assert_eq!(uuid.to_string().len(), 36);
}

#[cfg(feature = "reqwest")]
#[test]
fn http_client_can_be_overridden() {
    test_deps! {
        http_client.get => |_url| {
            Ok(clients::HttpResponse {
                status: 200,
                headers: Default::default(),
                body: b"ok".to_vec(),
            })
        },
    }

    let response = get::<clients::HttpClient>()
        .get("https://example.com".into())
        .expect("override should succeed");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"ok".to_vec());
}

fn unique_temp_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("system time should be after the unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("clients-{label}-{nanos}.tmp"))
}
