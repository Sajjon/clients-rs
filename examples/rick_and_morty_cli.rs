//! Small binary example that uses `dep` from a real `main` entry point.
//!
//! This example shows the intended "live app" shape:
//!
//! - declare an `ApiClient` with [`dep::client`]
//! - implement that client in terms of built-in live dependencies
//! - resolve the client from `main`
//! - keep the call sites small by binding built-ins with [`dep::deps`]
//!
//! The example talks to the public Rick and Morty API and caches successful
//! responses in the process temp directory so the fallback path is visible too.
//!
//! Run it with:
//!
//! ```bash
//! cargo run --example rick_and_morty_cli --features reqwest -- 1
//! ```
//!
//! If you omit the explicit id, the example picks one from a random byte. You
//! can also set:
//!
//! - `DEP_EXAMPLE_CHARACTER_ID` to choose the default character id
//! - `RICK_AND_MORTY_BASE_URL` to point at another compatible server

use dep::{
    Clock, Env, HttpClientError, HttpResponse, Random, client, clock, deps, env, filesystem, get,
    http_client, random,
};
use serde::Deserialize;
use std::error::Error;
use std::path::PathBuf;
use std::string::FromUtf8Error;
use thiserror::Error;

/// Small nested payload used by the Rick and Morty character response.
#[derive(Debug, Clone, Deserialize)]
struct CharacterPoint {
    /// Human-readable place name such as `"Earth (C-137)"`.
    name: String,
}

/// Subset of `GET /character/{id}` used by this example.
///
/// The API returns more fields than this, but the example keeps only enough
/// data to make the terminal output interesting and readable.
#[derive(Debug, Clone, Deserialize)]
struct Character {
    /// Stable numeric character identifier.
    id: u64,
    /// Character display name.
    name: String,
    /// Current status, for example `"Alive"` or `"Dead"`.
    status: String,
    /// Species name, for example `"Human"`.
    species: String,
    /// Gender string reported by the API.
    gender: String,
    /// Character origin summary.
    origin: CharacterPoint,
    /// Character current location summary.
    location: CharacterPoint,
    /// Remote image URL for the character.
    image: String,
}

/// Errors surfaced by the example `ApiClient`.
///
/// `thiserror` keeps the enum short while still preserving source errors for
/// HTTP, UTF-8, JSON, and filesystem failures.
#[derive(Debug, Error)]
enum ApiClientError {
    /// The built-in HTTP client failed before a valid success response arrived.
    #[error("HTTP request failed: {0}")]
    Http(#[from] HttpClientError),

    /// The remote server responded, but not with the expected success status.
    #[error("HTTP request to `{url}` returned status {status}")]
    UnexpectedStatus {
        /// Numeric status code returned by the server.
        status: u16,
        /// Requested URL.
        url: String,
    },

    /// The response body was not valid UTF-8.
    #[error("response body was not valid UTF-8: {0}")]
    Utf8(#[from] FromUtf8Error),

    /// The response body or cached fallback could not be decoded as JSON.
    #[error("response body was not valid JSON: {0}")]
    Json(#[from] serde_json::Error),

    /// Reading or writing the local cache failed.
    #[error("filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),

    /// Both the live request and the local cache fallback failed.
    #[error(
        "HTTP fetch failed ({http_error}) and cache `{}` was unavailable ({io_error})",
        cache_path.display()
    )]
    CacheUnavailable {
        /// Cache file that was attempted.
        cache_path: PathBuf,
        /// Original HTTP failure rendered as text.
        http_error: String,
        /// Cache read failure rendered as text.
        io_error: String,
    },
}

client! {
    /// API client used by the example binary.
    ///
    /// In a larger program this would be the dependency that the rest of the
    /// application talks to, rather than reaching for `http_client` directly.
    pub struct ApiClient as api_client {
        /// Fetches one Rick and Morty character by id.
        ///
        /// The live implementation uses the built-in `http_client`,
        /// `filesystem`, `env`, `random`, and `clock` dependencies.
        pub fn fetch_character(id: u64) -> Result<Character, ApiClientError> = fetch_character_live;
    }
}

/// Live implementation for [`ApiClient::fetch_character`].
///
/// This is the heart of the example. It intentionally pulls together multiple
/// built-ins in one place:
///
/// - `clock.now` for timestamped log lines
/// - `env.var` and `env.temp_dir` for process configuration
/// - `random.next_u64` for an ad hoc request tag
/// - `filesystem.read_string` and `filesystem.write_string` for cache fallback
/// - `http_client.get` for the actual API request
///
/// The body stays compact because [`dep::deps`] binds those methods to local
/// function pointers once at the top of the function.
fn fetch_character_live(id: u64) -> Result<Character, ApiClientError> {
    deps! {
        current_time = clock.now,
        temp_dir = env.temp_dir,
        env_var = env.var,
        next_random = random.next_u64,
        write_string = filesystem.write_string,
        read_string = filesystem.read_string,
        http_get = http_client.get,
    }

    let base_url = env_var("RICK_AND_MORTY_BASE_URL".to_string())
        .unwrap_or_else(|| "https://rickandmortyapi.com/api".to_string());
    let url = format!("{}/character/{id}", base_url.trim_end_matches('/'));
    let cache_path = temp_dir().join(format!("dep-example-rick-and-morty-character-{id}.json"));
    let request_tag = next_random();

    eprintln!(
        "[{request_tag}] GET {url} at {:?} (cache: {})",
        current_time(),
        cache_path.display()
    );

    match http_get(url.clone()) {
        Ok(HttpResponse {
            status: 200, body, ..
        }) => {
            let body = String::from_utf8(body)?;
            write_string(cache_path.clone(), body.clone())?;
            Ok(serde_json::from_str::<Character>(&body)?)
        }
        Ok(HttpResponse { status, .. }) => Err(ApiClientError::UnexpectedStatus { status, url }),
        Err(http_error) => {
            eprintln!(
                "[{request_tag}] HTTP failed ({http_error}); falling back to {}",
                cache_path.display()
            );
            let cached = read_string(cache_path.clone()).map_err(|io_error| {
                ApiClientError::CacheUnavailable {
                    cache_path,
                    http_error: http_error.to_string(),
                    io_error: io_error.to_string(),
                }
            })?;
            Ok(serde_json::from_str::<Character>(&cached)?)
        }
    }
}

/// Chooses the character id used by the CLI when the user did not pass one.
///
/// Resolution order:
///
/// 1. `DEP_EXAMPLE_CHARACTER_ID` if it exists and parses as `u64`
/// 2. one random byte from the built-in `Random` client, clamped to at least 1
fn choose_character_id() -> u64 {
    let env = get::<Env>();

    if let Some(value) = env.var("DEP_EXAMPLE_CHARACTER_ID".to_string()) {
        if let Ok(id) = value.parse::<u64>() {
            return id;
        }
    }

    let byte = get::<Random>()
        .fill_bytes(1)
        .into_iter()
        .next()
        .unwrap_or(1);
    u64::from(byte.max(1))
}

/// Entry point for the example binary.
///
/// The point of `main` is intentionally simple: resolve a few live dependencies,
/// print some context, fetch one character through `ApiClient`, and render the
/// result. That keeps the example focused on how `dep` is used in a normal
/// executable rather than on CLI framework setup.
fn main() -> Result<(), Box<dyn Error>> {
    let character_id = std::env::args()
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_else(choose_character_id);

    println!("dep example: Rick and Morty CLI");
    println!("started at: {:?}", get::<Clock>().now());
    println!("temp dir: {}", get::<Env>().temp_dir().display());
    println!("character id: {character_id}");

    let character = get::<ApiClient>().fetch_character(character_id)?;

    println!();
    println!("Character #{}", character.id);
    println!("Name: {}", character.name);
    println!("Status: {}", character.status);
    println!("Species: {}", character.species);
    println!("Gender: {}", character.gender);
    println!("Origin: {}", character.origin.name);
    println!("Location: {}", character.location.name);
    println!("Image: {}", character.image);
    println!();
    println!("Set `DEP_EXAMPLE_CHARACTER_ID` to choose the default character id.");
    println!("Set `RICK_AND_MORTY_BASE_URL` to point the client at another compatible server.");

    Ok(())
}
