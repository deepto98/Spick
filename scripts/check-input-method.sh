#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Spick Input checks can only run on macOS." >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd "${script_dir}/.." && pwd)"

unset APPLE_SIGNING_IDENTITY
unset SPICK_INPUT_TEAM_ID
unset SPICK_INPUT_ALLOW_UNSAFE_ADHOC_BUILD
unset SPICK_INPUT_ALLOW_UNSAFE_ADHOC_INSTALL

bash -n \
  "${script_dir}/build-input-method-desktop.sh" \
  "${script_dir}/build-input-method.sh" \
  "${script_dir}/check-input-method.sh" \
  "${script_dir}/install-input-method.sh" \
  "${script_dir}/preflight-input-method-compatibility.sh" \
  "${script_dir}/run-input-method-compatibility.sh" \
  "${script_dir}/lib/input-method-signing.sh"

if /usr/bin/grep -Eq \
  'spick-input-source-tool|prepare-install|register-and-enable|register-and-select' \
  "${script_dir}/preflight-input-method-compatibility.sh"; then
  echo "The read-only compatibility preflight references a multi-capability input-source tool." >&2
  exit 1
fi
if ! /usr/bin/grep -Fq 'argument.contains("input-method-compatibility")' \
  "${project_dir}/src-tauri/src/main.rs"; then
  echo "Normal desktop builds do not explicitly reject compatibility commands." >&2
  exit 1
fi
if ! /usr/bin/grep -Fq 'if (argc != 1)' \
  "${project_dir}/macos-input-method/Sources/main.m"; then
  echo "Spick Input does not reject unknown command-line arguments before starting its broker." >&2
  exit 1
fi

SPICK_INPUT_SIGNING_MODE=check "${script_dir}/build-input-method.sh"

if SPICK_INPUT_SIGNING_MODE=unsafe-adhoc \
  "${script_dir}/build-input-method.sh" >/dev/null 2>&1; then
  echo "Unsafe input-method builds worked without explicit confirmation." >&2
  exit 1
fi
if SPICK_INPUT_SIGNING_MODE=development \
  "${script_dir}/build-input-method.sh" >/dev/null 2>&1; then
  echo "A signed input-method build worked without an identity and Team ID." >&2
  exit 1
fi

cargo test --manifest-path "${project_dir}/src-tauri/Cargo.toml" \
  --features macos-input-method-prototype
cargo clippy --manifest-path "${project_dir}/src-tauri/Cargo.toml" \
  --all-targets --features macos-input-method-prototype -- -D warnings
cargo test --manifest-path "${project_dir}/src-tauri/Cargo.toml" \
  --features macos-input-method-compatibility-harness
cargo clippy --manifest-path "${project_dir}/src-tauri/Cargo.toml" \
  --all-targets --features macos-input-method-compatibility-harness -- -D warnings

SPICK_INPUT_SIGNING_MODE=unsafe-adhoc \
SPICK_INPUT_ALLOW_UNSAFE_ADHOC_BUILD=YES \
  "${script_dir}/build-input-method.sh"
cargo test --manifest-path "${project_dir}/src-tauri/Cargo.toml" \
  --features macos-input-method-unsafe-dev-peers
cargo clippy --manifest-path "${project_dir}/src-tauri/Cargo.toml" \
  --all-targets --features macos-input-method-unsafe-dev-peers -- -D warnings
cargo test --manifest-path "${project_dir}/src-tauri/Cargo.toml" \
  --features macos-input-method-compatibility-harness,macos-input-method-unsafe-dev-peers
cargo clippy --manifest-path "${project_dir}/src-tauri/Cargo.toml" \
  --all-targets \
  --features macos-input-method-compatibility-harness,macos-input-method-unsafe-dev-peers \
  -- -D warnings

# Leave the ordinary target artifact in its secure-policy check mode.
SPICK_INPUT_SIGNING_MODE=check "${script_dir}/build-input-method.sh"
