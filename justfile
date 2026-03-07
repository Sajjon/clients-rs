set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Run the full local verification suite used by CI.
ci: typos clippy test

# Check spelling across the repository.
typos:
    typos

# Lint all targets and features with warnings promoted to errors.
clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Run the full test suite across the workspace and all features.
test:
    cargo test --workspace --all-features

# Install the git pre-commit hook and its environments.
precommit-install:
    pre-commit install --install-hooks

# Run all configured pre-commit hooks against the full repository.
precommit:
    pre-commit run --all-files

# Package both publishable crates locally without uploading them.
#
# The proc-macro crate can be packaged locally. The runtime crate depends on the
# proc-macro being visible in the registry, so the GitHub publish workflow does
# the real `cargo publish --dry-run` validation for both crates against crates.io.
publish-dry-run:
    cargo package -p dep-macros --allow-dirty
    @echo "Skipping local dep dry-run; use the publish workflow for the registry-backed dry-run."

# Publish the proc-macro crate first, then the runtime crate.
publish:
    #!/usr/bin/env bash
    set -euo pipefail
    just ci
    cargo publish -p dep-macros --token "${CARGO_REGISTRY_TOKEN:?set CARGO_REGISTRY_TOKEN}"
    for attempt in 1 2 3 4 5; do
      if cargo publish -p dep --token "${CARGO_REGISTRY_TOKEN:?set CARGO_REGISTRY_TOKEN}"; then
        exit 0
      fi
      echo "dep publish did not succeed yet, retrying in 30 seconds..."
      sleep 30
    done
    echo "dep publish failed after 5 attempts" >&2
    exit 1
