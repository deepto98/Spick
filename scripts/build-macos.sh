#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Spick macOS bundles must be built on macOS." >&2
  exit 1
fi
if [[ "$#" -ne 1 || ( "$1" != "local" && "$1" != "release" ) ]]; then
  echo "Usage: $0 local|release" >&2
  exit 64
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd "${script_dir}/.." && pwd)"
mode="$1"
cd "${project_dir}"

if [[ "${mode}" == "local" ]]; then
  # Never let credentials inherited from the shell upload a local artifact.
  export APPLE_SIGNING_IDENTITY="-"
  unset APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID
  unset APPLE_API_ISSUER APPLE_API_KEY APPLE_API_KEY_PATH
else
  if [[ -z "${APPLE_SIGNING_IDENTITY:-}" || "${APPLE_SIGNING_IDENTITY}" == "-" ]]; then
    echo "Set APPLE_SIGNING_IDENTITY to a Developer ID Application identity." >&2
    exit 1
  fi
  identities="$(security find-identity -v -p codesigning)"
  if [[ "${identities}" != *\"${APPLE_SIGNING_IDENTITY}\"* ||
        "${APPLE_SIGNING_IDENTITY}" != Developer\ ID\ Application:* ]]; then
    echo "APPLE_SIGNING_IDENTITY is not an installed Developer ID Application identity." >&2
    exit 1
  fi
  has_api_credentials=0
  if [[ -n "${APPLE_API_ISSUER:-}" && -n "${APPLE_API_KEY:-}" &&
        -n "${APPLE_API_KEY_PATH:-}" && -f "${APPLE_API_KEY_PATH}" ]]; then
    has_api_credentials=1
  fi
  has_apple_id_credentials=0
  if [[ -n "${APPLE_ID:-}" && -n "${APPLE_PASSWORD:-}" &&
        -n "${APPLE_TEAM_ID:-}" ]]; then
    has_apple_id_credentials=1
  fi
  if [[ "${has_api_credentials}" -ne 1 && "${has_apple_id_credentials}" -ne 1 ]]; then
    echo "Set App Store Connect API credentials or APPLE_ID, APPLE_PASSWORD, and APPLE_TEAM_ID for notarization." >&2
    exit 1
  fi
fi

npm run tauri -- build --bundles app,dmg

app="${project_dir}/src-tauri/target/release/bundle/macos/Spick.app"
dmg_dir="${project_dir}/src-tauri/target/release/bundle/dmg"
if [[ ! -d "${app}" || -L "${app}" || ! -O "${app}" ]]; then
  echo "Tauri did not produce a safe Spick.app bundle." >&2
  exit 1
fi
/usr/bin/codesign --verify --deep --strict "${app}"
if ! find "${dmg_dir}" -maxdepth 1 -type f -name '*.dmg' -print -quit | grep -q .; then
  echo "Tauri did not produce a DMG." >&2
  exit 1
fi
if [[ "${mode}" == "release" ]]; then
  /usr/sbin/spctl --assess --type execute --verbose=2 "${app}"
fi

echo "Built Spick.app and DMG in src-tauri/target/release/bundle (${mode})."
