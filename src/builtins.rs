//! Small built-in dependency clients for common system interactions.
//!
//! These clients intentionally stay narrow. They cover common edges where an
//! application benefits from overridable behavior, while keeping the default
//! crate free from heavyweight optional dependencies.

#[cfg(feature = "uuid")]
use ::uuid::Uuid as GeneratedUuid;
#[cfg(feature = "reqwest")]
use std::collections::BTreeMap;
#[cfg(feature = "reqwest")]
use std::fmt;
use std::io::Error as IoError;
use std::path::PathBuf;
#[cfg(feature = "reqwest")]
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

/// Returns the current wall-clock time.
fn clock_now() -> SystemTime {
    SystemTime::now()
}

/// Waits for `duration` by blocking the current thread.
///
/// This powers [`Clock::sleep`] without tying the crate to a particular async
/// runtime.
async fn clock_sleep(duration: Duration) {
    std::thread::sleep(duration);
}

crate::client! {
    /// Dependency client for wall-clock time and coarse sleeping.
    ///
    /// `Clock` intentionally exposes only a small surface. It is enough for
    /// timestamps and simple delays without introducing a runtime-specific
    /// abstraction.
    ///
    /// # Examples
    ///
    /// ```
    /// let now = clients::get::<clients::Clock>().now();
    /// assert!(now <= std::time::SystemTime::now());
    /// ```
    pub struct Clock as clock {
        /// Returns the current wall-clock time according to the host system.
        pub fn now() -> SystemTime = clock_now;

        /// Resolves after at least `duration`.
        ///
        /// The built-in implementation blocks the current thread. It is useful
        /// for simple programs and tests, but if you need runtime-aware
        /// sleeping semantics you should override this client with your own
        /// implementation.
        ///
        /// ```no_run
        /// # use std::time::Duration;
        /// # fn main() {
        /// #     fn block_on<F: std::future::Future<Output = ()>>(_future: F) {}
        /// block_on(clients::get::<clients::Clock>().sleep(Duration::from_millis(10)));
        /// # }
        /// ```
        pub async fn sleep(duration: Duration) -> () = clock_sleep;
    }
}

/// Reads an environment variable, returning `None` when it is missing or
/// invalid Unicode.
fn env_var(name: String) -> Option<String> {
    std::env::var(name).ok()
}

/// Returns the process current working directory.
fn env_current_dir() -> Result<PathBuf, IoError> {
    std::env::current_dir()
}

/// Returns the process temporary directory.
fn env_temp_dir() -> PathBuf {
    std::env::temp_dir()
}

crate::client! {
    /// Dependency client for basic process-environment access.
    ///
    /// `Env` wraps a few commonly needed pieces of process state so they can be
    /// overridden in tests.
    ///
    /// # Examples
    ///
    /// ```
    /// let env = clients::get::<clients::Env>();
    /// assert!(env.temp_dir().is_absolute());
    /// assert_eq!(env.var("__DEP_MISSING_ENV_EXAMPLE__".into()), None);
    /// ```
    pub struct Env as env {
        /// Returns the value of `name` when it exists and contains valid
        /// Unicode.
        pub fn var(name: String) -> Option<String> = env_var;

        /// Returns the current working directory of the running process.
        pub fn current_dir() -> Result<PathBuf, IoError> = env_current_dir;

        /// Returns the operating system's preferred temporary directory.
        pub fn temp_dir() -> PathBuf = env_temp_dir;
    }
}

/// Produces one pseudo-random `u64` using the thread-local RNG.
fn random_u64() -> u64 {
    ::rand::random::<u64>()
}

/// Fills a buffer with `len` pseudo-random bytes.
fn random_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0; len];
    let mut rng = ::rand::thread_rng();
    ::rand::RngCore::fill_bytes(&mut rng, &mut bytes);
    bytes
}

crate::client! {
    /// Dependency client for pseudo-random data.
    ///
    /// The default implementation is backed by the `rand` crate's thread-local
    /// RNG, but tests can replace it deterministically.
    ///
    /// # Examples
    ///
    /// ```
    /// let random = clients::get::<clients::Random>();
    /// let bytes = random.fill_bytes(8);
    ///
    /// assert_eq!(bytes.len(), 8);
    /// let _ = random.next_u64();
    /// ```
    pub struct Random as random {
        /// Returns one pseudo-random `u64`.
        pub fn next_u64() -> u64 = random_u64;

        /// Returns `len` pseudo-random bytes.
        pub fn fill_bytes(len: usize) -> Vec<u8> = random_bytes;
    }
}

/// Reads the full contents of `path` as raw bytes.
fn filesystem_read(path: PathBuf) -> Result<Vec<u8>, IoError> {
    std::fs::read(path)
}

/// Reads the full contents of `path` as UTF-8 text.
fn filesystem_read_string(path: PathBuf) -> Result<String, IoError> {
    std::fs::read_to_string(path)
}

/// Writes raw bytes to `path`, replacing any existing contents.
fn filesystem_write(path: PathBuf, contents: Vec<u8>) -> Result<(), IoError> {
    std::fs::write(path, contents)
}

/// Writes a UTF-8 string to `path`, replacing any existing contents.
fn filesystem_write_string(path: PathBuf, contents: String) -> Result<(), IoError> {
    std::fs::write(path, contents)
}

crate::client! {
    /// Dependency client for small read and write file operations.
    ///
    /// This client intentionally mirrors the corresponding `std::fs` helpers
    /// instead of modeling a full virtual file system.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::{SystemTime, UNIX_EPOCH};
    ///
    /// let filesystem = clients::get::<clients::Filesystem>();
    /// let unique = SystemTime::now()
    ///     .duration_since(UNIX_EPOCH)
    ///     .expect("system time should be after the unix epoch")
    ///     .as_nanos();
    /// let path = std::env::temp_dir().join(format!("clients-doc-filesystem-{unique}.txt"));
    ///
    /// filesystem
    ///     .write_string(path.clone(), "hello".to_string())
    ///     .expect("write should succeed");
    /// assert_eq!(
    ///     filesystem.read_string(path.clone()).expect("read should succeed"),
    ///     "hello"
    /// );
    ///
    /// std::fs::remove_file(path).expect("temp file should be removable");
    /// ```
    pub struct Filesystem as filesystem {
        /// Reads the file at `path` into memory as raw bytes.
        pub fn read(path: PathBuf) -> Result<Vec<u8>, IoError> = filesystem_read;

        /// Reads the file at `path` into memory as a UTF-8 string.
        pub fn read_string(path: PathBuf) -> Result<String, IoError> = filesystem_read_string;

        /// Writes `contents` to `path`, replacing any existing file.
        pub fn write(path: PathBuf, contents: Vec<u8>) -> Result<(), IoError> = filesystem_write;

        /// Writes `contents` to `path`, replacing any existing file.
        pub fn write_string(path: PathBuf, contents: String) -> Result<(), IoError> = filesystem_write_string;
    }
}

/// Generates a version 4 UUID.
#[cfg(feature = "uuid")]
fn generate_uuid() -> GeneratedUuid {
    GeneratedUuid::new_v4()
}

#[cfg(feature = "uuid")]
crate::client! {
    /// Dependency client for generating UUID values.
    ///
    /// This client is available when the crate's `uuid` feature is enabled.
    ///
    /// # Examples
    ///
    /// ```
    /// let value = clients::get::<clients::Uuid>().generate();
    /// assert_eq!(value.to_string().len(), 36);
    /// ```
    pub struct Uuid as uuid {
        /// Generates one UUID value.
        pub fn generate() -> GeneratedUuid = generate_uuid;
    }
}

/// The HTTP method used by [`HttpRequest`].
#[cfg(feature = "reqwest")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpMethod {
    /// `GET`
    Get,
    /// `POST`
    Post,
    /// `PUT`
    Put,
    /// `PATCH`
    Patch,
    /// `DELETE`
    Delete,
    /// `HEAD`
    Head,
    /// `OPTIONS`
    Options,
}

#[cfg(feature = "reqwest")]
impl HttpMethod {
    /// Converts this method into the corresponding `reqwest` value.
    fn as_reqwest(&self) -> ::reqwest::Method {
        match self {
            Self::Get => ::reqwest::Method::GET,
            Self::Post => ::reqwest::Method::POST,
            Self::Put => ::reqwest::Method::PUT,
            Self::Patch => ::reqwest::Method::PATCH,
            Self::Delete => ::reqwest::Method::DELETE,
            Self::Head => ::reqwest::Method::HEAD,
            Self::Options => ::reqwest::Method::OPTIONS,
        }
    }
}

/// A single HTTP request executed by [`HttpClient`].
#[cfg(feature = "reqwest")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    /// The HTTP method to issue.
    pub method: HttpMethod,
    /// The absolute URL to request.
    pub url: String,
    /// Request headers keyed by header name.
    pub headers: BTreeMap<String, String>,
    /// Raw request body bytes.
    pub body: Vec<u8>,
}

#[cfg(feature = "reqwest")]
impl HttpRequest {
    /// Builds a `GET` request with an empty body and no headers.
    ///
    /// ```
    /// let request = clients::HttpRequest::get("https://example.com");
    ///
    /// assert_eq!(request.method, clients::HttpMethod::Get);
    /// assert_eq!(request.url, "https://example.com");
    /// assert!(request.headers.is_empty());
    /// assert!(request.body.is_empty());
    /// ```
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: HttpMethod::Get,
            url: url.into(),
            headers: BTreeMap::new(),
            body: Vec::new(),
        }
    }

    /// Builds a `POST` request with the supplied raw body and no headers.
    ///
    /// ```
    /// let request = clients::HttpRequest::post("https://example.com", b"hello".to_vec());
    ///
    /// assert_eq!(request.method, clients::HttpMethod::Post);
    /// assert_eq!(request.body, b"hello".to_vec());
    /// ```
    pub fn post(url: impl Into<String>, body: Vec<u8>) -> Self {
        Self {
            method: HttpMethod::Post,
            url: url.into(),
            headers: BTreeMap::new(),
            body,
        }
    }
}

/// A fully materialized HTTP response returned by [`HttpClient`].
#[cfg(feature = "reqwest")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    /// The numeric HTTP status code.
    pub status: u16,
    /// Response headers keyed by header name.
    pub headers: BTreeMap<String, String>,
    /// Raw response body bytes.
    pub body: Vec<u8>,
}

/// Errors returned by the built-in [`HttpClient`].
#[cfg(feature = "reqwest")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpClientError {
    /// Request construction failed before the request could be sent.
    BuildRequest(String),
    /// The underlying transport failed while sending or receiving.
    Transport(String),
}

#[cfg(feature = "reqwest")]
impl fmt::Display for HttpClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BuildRequest(error) => write!(formatter, "failed to build HTTP request: {error}"),
            Self::Transport(error) => write!(formatter, "HTTP transport error: {error}"),
        }
    }
}

#[cfg(feature = "reqwest")]
impl std::error::Error for HttpClientError {}

/// Returns the shared blocking `reqwest` client used by the built-in HTTP
/// dependency.
#[cfg(feature = "reqwest")]
fn reqwest_client() -> &'static ::reqwest::blocking::Client {
    static CLIENT: OnceLock<::reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(::reqwest::blocking::Client::new)
}

/// Executes `request` through the shared blocking `reqwest` client.
#[cfg(feature = "reqwest")]
fn execute_http_request(request: HttpRequest) -> Result<HttpResponse, HttpClientError> {
    let client = reqwest_client();
    let mut builder = client.request(request.method.as_reqwest(), request.url);

    for (name, value) in request.headers {
        builder = builder.header(name, value);
    }

    if !request.body.is_empty() {
        builder = builder.body(request.body);
    }

    let response = builder
        .send()
        .map_err(|error| HttpClientError::Transport(error.to_string()))?;

    let status = response.status().as_u16();
    let mut headers = BTreeMap::new();
    for (name, value) in response.headers() {
        headers.insert(
            name.to_string(),
            value.to_str().unwrap_or_default().to_string(),
        );
    }

    let body = response
        .bytes()
        .map_err(|error| HttpClientError::Transport(error.to_string()))?
        .to_vec();

    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

#[cfg(feature = "reqwest")]
crate::client! {
    /// Overridable HTTP client backed by `reqwest`.
    ///
    /// The built-in implementation uses `reqwest::blocking::Client`, which
    /// keeps the dependency surface small and works in both sync code and
    /// tests. As with every other client in `clients`, you can override just the
    /// pieces you need.
    ///
    /// # Examples
    ///
    /// ```
    /// use clients::{HttpClient, HttpRequest, HttpResponse, get, test_deps};
    /// use clients::http_client;
    ///
    /// test_deps! {
    ///     http_client.execute => |request| {
    ///         assert_eq!(request.url, "https://example.com");
    ///         Ok(HttpResponse {
    ///             status: 200,
    ///             headers: Default::default(),
    ///             body: b"ok".to_vec(),
    ///         })
    ///     },
    /// }
    ///
    /// let response = get::<HttpClient>()
    ///     .execute(HttpRequest::get("https://example.com"))
    ///     .expect("override should succeed");
    /// assert_eq!(response.body, b"ok".to_vec());
    /// ```
    pub struct HttpClient as http_client {
        /// Executes an arbitrary HTTP request.
        pub fn execute(request: HttpRequest) -> Result<HttpResponse, HttpClientError> = |request: HttpRequest| {
            execute_http_request(request)
        };

        /// Executes a `GET` request for `url`.
        pub fn get(url: String) -> Result<HttpResponse, HttpClientError> = |url: String| {
            execute_http_request(HttpRequest::get(url))
        };

        /// Executes a `POST` request for `url` with `body` as the raw body.
        pub fn post(url: String, body: Vec<u8>) -> Result<HttpResponse, HttpClientError> = |url: String, body: Vec<u8>| {
            execute_http_request(HttpRequest::post(url, body))
        };
    }
}
