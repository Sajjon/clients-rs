extern crate self as dep;

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::mem::{self, MaybeUninit};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};
use std::thread;

pub use dep_macros::{Depends, client};

type DependencyMap = HashMap<TypeId, Box<dyn Any + Send + Sync>>;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyError {
    Missing(&'static str),
    Message(&'static str),
    Owned(String),
}

impl DependencyError {
    pub const fn missing(path: &'static str) -> Self {
        Self::Missing(path)
    }

    pub const fn message(message: &'static str) -> Self {
        Self::Message(message)
    }
}

impl fmt::Display for DependencyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(path) => {
                write!(formatter, "missing dependency implementation for `{path}`")
            }
            Self::Message(message) => formatter.write_str(message),
            Self::Owned(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for DependencyError {}

pub trait Dependency: Clone + Send + Sync + 'static {
    fn live() -> Self;
}

pub fn boxed<Fut>(future: Fut) -> BoxFuture<Fut::Output>
where
    Fut: Future + Send + 'static,
{
    Box::pin(future)
}

pub fn get<D>() -> D
where
    D: Dependency,
{
    current_override::<D>().unwrap_or_else(D::live)
}

pub fn unimplemented_dependency(path: &'static str) -> ! {
    panic!("dependency `{path}` was used without a live implementation or a test override")
}

fn active_overrides() -> &'static RwLock<Vec<DependencyMap>> {
    static ACTIVE_OVERRIDES: OnceLock<RwLock<Vec<DependencyMap>>> = OnceLock::new();
    ACTIVE_OVERRIDES.get_or_init(|| RwLock::new(Vec::new()))
}

fn current_override<D>() -> Option<D>
where
    D: Dependency,
{
    let overrides = active_overrides()
        .read()
        .expect("dependency override lock poisoned");
    for layer in overrides.iter().rev() {
        if let Some(value) = layer.get(&TypeId::of::<D>()) {
            let dependency = value
                .downcast_ref::<D>()
                .expect("dependency override stored with the wrong type");
            return Some(dependency.clone());
        }
    }
    None
}

fn push_overrides(entries: DependencyMap) {
    active_overrides()
        .write()
        .expect("dependency override lock poisoned")
        .push(entries);
}

fn pop_overrides() {
    let popped = active_overrides()
        .write()
        .expect("dependency override lock poisoned")
        .pop();

    assert!(popped.is_some(), "dependency override stack underflow");
}

fn test_scope_lock() -> &'static AtomicBool {
    static TEST_SCOPE_LOCK: AtomicBool = AtomicBool::new(false);
    &TEST_SCOPE_LOCK
}

fn acquire_test_lock() {
    while test_scope_lock()
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        thread::yield_now();
    }
}

fn release_test_lock() {
    test_scope_lock().store(false, Ordering::Release);
}

pub struct OverrideBuilder {
    entries: DependencyMap,
}

impl Default for OverrideBuilder {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl OverrideBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set<D>(&mut self, dependency: D) -> &mut Self
    where
        D: Dependency,
    {
        self.entries.insert(
            TypeId::of::<D>(),
            Box::new(dependency) as Box<dyn Any + Send + Sync>,
        );
        self
    }

    pub fn update<D, F>(&mut self, update: F) -> &mut Self
    where
        D: Dependency,
        F: FnOnce(D) -> D,
    {
        let current = self.take_or_resolve::<D>();
        self.set(update(current))
    }

    pub fn enter(self) -> OverrideGuard {
        push_overrides(self.entries);
        OverrideGuard {
            release_test_lock: false,
        }
    }

    pub fn enter_test(self) -> OverrideGuard {
        acquire_test_lock();
        push_overrides(self.entries);
        OverrideGuard {
            release_test_lock: true,
        }
    }

    fn take_or_resolve<D>(&mut self) -> D
    where
        D: Dependency,
    {
        if let Some(entry) = self.entries.remove(&TypeId::of::<D>()) {
            *entry
                .downcast::<D>()
                .expect("dependency override stored with the wrong type")
        } else {
            get::<D>()
        }
    }
}

pub struct OverrideGuard {
    release_test_lock: bool,
}

impl Drop for OverrideGuard {
    fn drop(&mut self) {
        pop_overrides();
        if self.release_test_lock {
            release_test_lock();
        }
    }
}

fn assert_non_capturing<F>() {
    assert!(
        mem::size_of::<F>() == 0,
        "dependency implementations must be non-capturing closures or function items"
    );
}

unsafe fn resurrect_zst<F>() -> F {
    debug_assert_eq!(mem::size_of::<F>(), 0);
    unsafe { MaybeUninit::<F>::uninit().assume_init() }
}

macro_rules! define_erasers {
    ($( $sync_name:ident, $async_name:ident, ( $( $arg:ident : $arg_ty:ident ),* ) );* $(;)?) => {
        $(
            #[doc(hidden)]
            pub fn $sync_name<F, R $(, $arg_ty)*>(_: F) -> fn($( $arg_ty ),*) -> R
            where
                F: Fn($( $arg_ty ),*) -> R + Copy + 'static,
            {
                assert_non_capturing::<F>();

                fn trampoline<F, R $(, $arg_ty)*>($( $arg : $arg_ty ),*) -> R
                where
                    F: Fn($( $arg_ty ),*) -> R + Copy + 'static,
                {
                    let function: F = unsafe { resurrect_zst() };
                    function($( $arg ),*)
                }

                trampoline::<F, R $(, $arg_ty)*>
            }

            #[doc(hidden)]
            pub fn $async_name<F, Fut, R $(, $arg_ty)*>(_: F) -> fn($( $arg_ty ),*) -> BoxFuture<R>
            where
                F: Fn($( $arg_ty ),*) -> Fut + Copy + 'static,
                Fut: Future<Output = R> + Send + 'static,
            {
                assert_non_capturing::<F>();

                fn trampoline<F, Fut, R $(, $arg_ty)*>($( $arg : $arg_ty ),*) -> BoxFuture<R>
                where
                    F: Fn($( $arg_ty ),*) -> Fut + Copy + 'static,
                    Fut: Future<Output = R> + Send + 'static,
                {
                    let function: F = unsafe { resurrect_zst() };
                    Box::pin(function($( $arg ),*))
                }

                trampoline::<F, Fut, R $(, $arg_ty)*>
            }
        )*
    };
}

define_erasers! {
    erase_sync_0, erase_async_0, ();
    erase_sync_1, erase_async_1, (arg0: A0);
    erase_sync_2, erase_async_2, (arg0: A0, arg1: A1);
    erase_sync_3, erase_async_3, (arg0: A0, arg1: A1, arg2: A2);
    erase_sync_4, erase_async_4, (arg0: A0, arg1: A1, arg2: A2, arg3: A3);
}

#[doc(hidden)]
#[macro_export]
macro_rules! __dep_to_sync_fn {
    (() => $implementation:expr) => {
        $crate::erase_sync_0($implementation)
    };
    (($arg0:ident : $arg0_ty:ty) => $implementation:expr) => {
        $crate::erase_sync_1($implementation)
    };
    (($arg0:ident : $arg0_ty:ty, $arg1:ident : $arg1_ty:ty) => $implementation:expr) => {
        $crate::erase_sync_2($implementation)
    };
    (($arg0:ident : $arg0_ty:ty, $arg1:ident : $arg1_ty:ty, $arg2:ident : $arg2_ty:ty) => $implementation:expr) => {
        $crate::erase_sync_3($implementation)
    };
    (($arg0:ident : $arg0_ty:ty, $arg1:ident : $arg1_ty:ty, $arg2:ident : $arg2_ty:ty, $arg3:ident : $arg3_ty:ty) => $implementation:expr) => {
        $crate::erase_sync_4($implementation)
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __dep_to_async_fn {
    (() => $implementation:expr) => {
        $crate::erase_async_0($implementation)
    };
    (($arg0:ident : $arg0_ty:ty) => $implementation:expr) => {
        $crate::erase_async_1($implementation)
    };
    (($arg0:ident : $arg0_ty:ty, $arg1:ident : $arg1_ty:ty) => $implementation:expr) => {
        $crate::erase_async_2($implementation)
    };
    (($arg0:ident : $arg0_ty:ty, $arg1:ident : $arg1_ty:ty, $arg2:ident : $arg2_ty:ty) => $implementation:expr) => {
        $crate::erase_async_3($implementation)
    };
    (($arg0:ident : $arg0_ty:ty, $arg1:ident : $arg1_ty:ty, $arg2:ident : $arg2_ty:ty, $arg3:ident : $arg3_ty:ty) => $implementation:expr) => {
        $crate::erase_async_4($implementation)
    };
}

#[macro_export]
macro_rules! deps {
    () => {};
    ($binding:ident = $client:ident.$method:ident $(, $($rest:tt)*)?) => {
        let $binding = $client::$method::get();
        $crate::deps!($($($rest)*)?);
    };
}

#[macro_export]
macro_rules! test_deps {
    () => {
        let __dep_test_scope_guard = $crate::OverrideBuilder::new().enter_test();
        let _ = &__dep_test_scope_guard;
    };
    ($client:ident.$method:ident => $implementation:expr $(, $($rest:tt)*)?) => {
        let mut __dep_builder = $crate::OverrideBuilder::new();
        $client::$method::override_with(&mut __dep_builder, $implementation);
        $(
            $crate::__dep_test_deps_more!(__dep_builder, $($rest)*);
        )?
        let __dep_test_scope_guard = __dep_builder.enter_test();
        let _ = &__dep_test_scope_guard;
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __dep_test_deps_more {
    ($builder:ident, ) => {};
    ($builder:ident, $client:ident.$method:ident => $implementation:expr $(, $($rest:tt)*)?) => {
        $client::$method::override_with(&mut $builder, $implementation);
        $(
            $crate::__dep_test_deps_more!($builder, $($rest)*);
        )?
    };
}
