#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "The Spick macOS compatibility app can only be built on macOS." >&2
  exit 1
fi
if [[ "$#" -ne 1 || ( "$1" != "development" && "$1" != "unsafe-adhoc" ) ]]; then
  echo "Usage: $0 development|unsafe-adhoc" >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd "${script_dir}/.." && pwd)"
source "${script_dir}/lib/input-method-signing.sh"

signing_mode="$1"
spick_validate_input_signing_configuration "${signing_mode}"

feature="macos-input-method-prototype"
signing_identity="${APPLE_SIGNING_IDENTITY:--}"
if [[ "${signing_mode}" == "unsafe-adhoc" ]]; then
  feature="macos-input-method-unsafe-dev-peers"
fi

cd "${project_dir}"
npm run tauri -- build --debug --bundles app --features "${feature}"

app="${project_dir}/src-tauri/target/debug/bundle/macos/Spick.app"
executable="${app}/Contents/MacOS/spick-desktop"
if [[ ! -d "${app}" || -L "${app}" || ! -O "${app}" ||
      ! -f "${executable}" || ! -x "${executable}" || -L "${executable}" ||
      ! -O "${executable}" ]]; then
  echo "Tauri produced an app with an unsafe file shape." >&2
  exit 1
fi

/usr/bin/codesign --force --sign "${signing_identity}" \
  --identifier app.spick.desktop --options runtime --timestamp=none "${app}"
/usr/bin/codesign --verify --deep --strict "${app}"

display="$(spick_codesign_display "${app}")"
identifier="$(spick_codesign_value "${display}" "Identifier")"
team_id="$(spick_codesign_value "${display}" "TeamIdentifier")"
signature="$(spick_codesign_value "${display}" "Signature")"
if [[ "${identifier}" != "app.spick.desktop" || "${display}" != *"runtime)"* ]]; then
  echo "The compatibility app lacks its exact signing ID or hardened runtime." >&2
  exit 1
fi

if [[ "${signing_mode}" == "unsafe-adhoc" ]]; then
  if [[ "${signature}" != "adhoc" || "${team_id}" != "not set" ]]; then
    echo "The unsafe compatibility app must be ad-hoc signed with no Team ID." >&2
    exit 1
  fi
  /usr/bin/codesign --verify --deep --strict \
    -R='identifier "app.spick.desktop"' "${app}"
else
  if [[ "${signature}" == "adhoc" || "${team_id}" != "${SPICK_INPUT_TEAM_ID}" ]]; then
    echo "The compatibility app is not signed by the configured Team ID." >&2
    exit 1
  fi
  requirement="identifier \"app.spick.desktop\" and anchor apple generic and certificate leaf[subject.OU] = \"${SPICK_INPUT_TEAM_ID}\""
  /usr/bin/codesign --verify --deep --strict -R="${requirement}" "${app}"
fi

entitlements="$(/usr/bin/codesign -d --entitlements :- "${app}" 2>/dev/null || true)"
for dangerous_entitlement in \
  com.apple.security.get-task-allow \
  com.apple.security.cs.disable-library-validation \
  com.apple.security.cs.allow-dyld-environment-variables \
  com.apple.security.cs.disable-executable-page-protection \
  com.apple.security.cs.allow-unsigned-executable-memory; do
  if /usr/bin/grep -Fq "<key>${dangerous_entitlement}</key>" <<<"${entitlements}"; then
    echo "The compatibility app has a peer-authentication-breaking entitlement: ${dangerous_entitlement}." >&2
    exit 1
  fi
done

expected_auth_mode="secure"
if [[ "${signing_mode}" == "unsafe-adhoc" ]]; then
  expected_auth_mode="unsafe-adhoc"
fi
actual_auth_mode="$("${executable}" --print-input-method-peer-auth-mode)"
if [[ "${actual_auth_mode}" != "${expected_auth_mode}" ]]; then
  echo "The desktop app's compiled peer-authentication mode is inconsistent." >&2
  exit 1
fi

echo "Built and verified ${app} (${signing_mode})"
