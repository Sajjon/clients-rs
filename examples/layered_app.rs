//! Demonstrates deep dependency nesting without manual injection.
//!
//! This example shows the "layered architecture" pattern: an `App` at the top
//! holds a `Fetcher`, which holds an `Api`, which depends on an `HttpClient`
//! declared with `client!`. The only thing `main` needs to provide explicitly
//! is a `Config` (marked `#[arg]`). Every dependency-backed field resolves
//! automatically through the generated `Default` / `from_deps` chain.
//!
//! ```text
//! App
//! ├── config: Config          ← #[arg], passed by caller
//! └── fetcher: Fetcher        ← Default (auto)
//!     └── api: Api            ← Default (auto)
//!         └── http: Http      ← #[dep], resolved via get::<Http>()
//! ```
//!
//! Run it with:
//!
//! ```bash
//! cargo run --example layered_app
//! ```

use clients::{Depends, client, get};

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// Application log level. Intentionally does **not** implement `Default`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum LogLevel {
    Info,
    Debug,
}

/// Application configuration. Intentionally does **not** implement `Default`
/// or use `client!` — it is plain data that the caller must supply.
#[derive(Debug, Clone)]
struct Config {
    log_level: LogLevel,
    base_url: String,
}

// ---------------------------------------------------------------------------
// Dependency client (leaf)
// ---------------------------------------------------------------------------

// A minimal HTTP client dependency.
//
// In a real app this would wrap `reqwest` or similar; here the live
// implementation just returns a placeholder so the example compiles without
// network access.
client! {
    pub struct Http as http {
        pub fn get_text(url: String) -> Result<String, String> = |url: String| {
            Ok(format!("[stub response for {url}]"))
        };
    }
}

// ---------------------------------------------------------------------------
// Layers that compose via #[derive(Depends)]
// ---------------------------------------------------------------------------

/// Low-level API layer that uses the HTTP dependency directly.
#[derive(Depends)]
struct Api {
    #[dep]
    http: Http,
}

impl Api {
    fn fetch(&self, path: &str, base_url: &str) -> Result<String, String> {
        let url = format!("{}/{}", base_url.trim_end_matches('/'), path);
        self.http.get_text(url)
    }
}

/// Mid-level fetcher. It holds an `Api` as a plain field — no `#[dep]`
/// needed because `Api` already derives `Depends` (which generates `Default`).
#[derive(Depends)]
struct Fetcher {
    api: Api,
}

impl Fetcher {
    fn fetch_users(&self, base_url: &str) -> Result<String, String> {
        self.api.fetch("users", base_url)
    }
}

/// Top-level application struct.
///
/// - `config` is marked `#[arg]` because it has no `Default` and is not a
///   dependency client — the caller must supply it.
/// - `fetcher` is a plain field whose type (`Fetcher`) implements `Default`
///   through its own `Depends` derive, so it resolves automatically.
#[derive(Depends)]
struct App {
    #[arg]
    config: Config,
    fetcher: Fetcher,
}

impl App {
    fn start(&self) -> Result<(), String> {
        println!("[{:?}] App starting", self.config.log_level);
        println!("  base_url = {}", self.config.base_url);

        let users = self.fetcher.fetch_users(&self.config.base_url)?;
        println!("  users    = {users}");

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Entry point — note how simple construction is
// ---------------------------------------------------------------------------

fn main() -> Result<(), String> {
    // The only thing we build by hand is Config.
    // Http is resolved automatically 3 layers deep: App → Fetcher → Api → Http.
    let app = App::from_deps(Config {
        log_level: LogLevel::Info,
        base_url: "https://api.example.com".into(),
    });

    app.start()?;

    // You can also resolve individual clients if you need them:
    let http = get::<Http>();
    println!(
        "\ndirect http call = {}",
        http.get_text("https://other.example.com/health".into())?
    );

    Ok(())
}
