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
  "${script_dir}/lib/input-method-signing.sh"

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

SPICK_INPUT_SIGNING_MODE=unsafe-adhoc \
SPICK_INPUT_ALLOW_UNSAFE_ADHOC_BUILD=YES \
  "${script_dir}/build-input-method.sh"
cargo test --manifest-path "${project_dir}/src-tauri/Cargo.toml" \
  --features macos-input-method-unsafe-dev-peers
cargo clippy --manifest-path "${project_dir}/src-tauri/Cargo.toml" \
  --all-targets --features macos-input-method-unsafe-dev-peers -- -D warnings

# Leave the ordinary target artifact in its secure-policy check mode.
SPICK_INPUT_SIGNING_MODE=check "${script_dir}/build-input-method.sh"
