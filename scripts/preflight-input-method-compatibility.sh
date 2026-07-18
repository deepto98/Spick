#!/usr/bin/env bash
set -euo pipefail

# This preflight is observation-only. It deliberately does not build, copy,
# start either app UI or insertion broker, register, enable, disable, select,
# or deselect an input source.

if [[ "$#" -ne 0 ]]; then
  echo "Usage: $0" >&2
  exit 64
fi

if [[ "$(/usr/bin/uname -s)" != "Darwin" ]]; then
  echo "Spick input-method compatibility checks can only run on macOS." >&2
  exit 1
fi

script_dir="$(cd "$(/usr/bin/dirname "${BASH_SOURCE[0]}")" && pwd -P)"
project_dir="$(cd "${script_dir}/.." && pwd -P)"
fixture="${project_dir}/fixtures/input-method/browser-controls.html"
installed_bundle="${HOME}/Library/Input Methods/Spick Input.app"
installed_helper="${installed_bundle}/Contents/MacOS/SpickInput"
helper_info="${installed_bundle}/Contents/Info.plist"
desktop_app="${project_dir}/src-tauri/target/debug/bundle/macos/Spick.app"
desktop_executable="${desktop_app}/Contents/MacOS/spick-desktop"
desktop_info="${desktop_app}/Contents/Info.plist"
expected_helper_identifier="app.spick.desktop.input-method"
expected_desktop_identifier="app.spick.desktop"
readiness_issues=0

pass() {
  printf '  pass  %s\n' "$1"
}

note() {
  printf '  note  %s\n' "$1"
}

issue() {
  printf '  stop  %s\n' "$1"
  readiness_issues=$((readiness_issues + 1))
}

codesign_field() {
  local artifact="$1"
  local field="$2"
  local display
  display="$(LC_ALL=C /usr/bin/codesign -d --verbose=4 "${artifact}" 2>&1 || true)"
  /usr/bin/sed -n "s/^${field}=//p" <<<"${display}" | /usr/bin/head -n 1
}

has_forbidden_entitlement() {
  local artifact="$1"
  local entitlements
  entitlements="$(/usr/bin/codesign -d --entitlements :- "${artifact}" 2>/dev/null || true)"
  for entitlement in \
    com.apple.security.get-task-allow \
    com.apple.security.cs.disable-library-validation \
    com.apple.security.cs.allow-dyld-environment-variables \
    com.apple.security.cs.disable-executable-page-protection \
    com.apple.security.cs.allow-unsigned-executable-memory; do
    if /usr/bin/grep -Fq "<key>${entitlement}</key>" <<<"${entitlements}"; then
      return 0
    fi
  done
  return 1
}

printf 'Spick input-method compatibility preflight (read-only)\n\n'
printf 'System\n'
printf '  macOS %s · %s\n' "$(/usr/bin/sw_vers -productVersion)" "$(/usr/bin/uname -m)"
if [[ -d /System/Library/Frameworks/InputMethodKit.framework ]]; then
  pass "InputMethodKit is present."
else
  issue "InputMethodKit is not present on this system."
fi

printf '\nBrowser fixture\n'
if [[ ! -f "${fixture}" || -L "${fixture}" ]]; then
  issue "The local browser fixture is missing or is a symlink."
else
  fixture_is_complete=1
  for required_marker in \
    'id="plain-input"' \
    'id="multiline-textarea"' \
    'id="contenteditable-editor"' \
    'id="password-input"' \
    "default-src 'none'"; do
    if ! /usr/bin/grep -Fq "${required_marker}" "${fixture}"; then
      fixture_is_complete=0
      issue "The fixture is missing required marker: ${required_marker}"
    fi
  done
  if [[ "${fixture_is_complete}" -eq 1 ]]; then
    pass "The four-control offline fixture is complete and network-blocked."
    note "Open manually in each browser: ${fixture}"
  fi
fi

printf '\nInput-source state\n'
helper_ready=0
helper_version=""
helper_authentication_mode=""
if [[ ! -d "${installed_bundle}" || -L "${installed_bundle}" || ! -O "${installed_bundle}" ||
      ! -f "${installed_helper}" || ! -x "${installed_helper}" ||
      -L "${installed_helper}" || ! -O "${installed_helper}" ]]; then
  issue "The installed Spick Input bundle has an unsafe or missing file shape."
elif ! /usr/bin/codesign --verify --deep --strict "${installed_bundle}" >/dev/null 2>&1; then
  issue "The installed Spick Input bundle fails strict code-signature verification."
elif [[ "$(codesign_field "${installed_bundle}" Identifier)" != "${expected_helper_identifier}" ]]; then
  issue "The installed input method has an unexpected signing identifier."
elif has_forbidden_entitlement "${installed_bundle}"; then
  issue "The installed input method has a peer-authentication-breaking entitlement."
elif [[ ! -f "${helper_info}" || -L "${helper_info}" ||
        "$(/usr/libexec/PlistBuddy -c 'Print :SpickInputInspectionProtocol' "${helper_info}" 2>/dev/null || true)" != "1" ]]; then
  issue "The installed input method lacks the signed read-only inspection protocol."
else
  helper_ready=1
  helper_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "${helper_info}" 2>/dev/null || true)"
  helper_authentication_mode="$(/usr/libexec/PlistBuddy -c 'Print :SpickPeerAuthenticationMode' "${helper_info}" 2>/dev/null || true)"
  pass "The installed input method has a valid signature, exact identifier, and safe entitlements."
fi
printf '\nDesktop compatibility app\n'
desktop_ready=0
desktop_version=""
if [[ ! -d "${desktop_app}" || -L "${desktop_app}" || ! -O "${desktop_app}" ||
      ! -f "${desktop_executable}" || ! -x "${desktop_executable}" ||
      -L "${desktop_executable}" || ! -O "${desktop_executable}" ]]; then
  issue "The signed desktop compatibility app is not present at ${desktop_app}."
elif ! /usr/bin/codesign --verify --deep --strict "${desktop_app}" >/dev/null 2>&1; then
  issue "The desktop compatibility app does not pass strict code-signature verification."
elif [[ "$(codesign_field "${desktop_app}" Identifier)" != "${expected_desktop_identifier}" ]]; then
  issue "The desktop compatibility app has an unexpected signing identifier."
elif has_forbidden_entitlement "${desktop_app}"; then
  issue "The desktop compatibility app has a peer-authentication-breaking entitlement."
elif [[ ! -f "${desktop_info}" || -L "${desktop_info}" ||
        "$(/usr/libexec/PlistBuddy -c 'Print :SpickInputCompatibilityMode' "${desktop_info}" 2>/dev/null || true)" != "fixed-fixture-v1" ]]; then
  issue "The signed desktop bundle does not contain the compatibility marker."
else
  desktop_ready=1
  desktop_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "${desktop_info}" 2>/dev/null || true)"
  pass "The desktop compatibility app has a valid code signature and exact identifier."
  pass "The desktop bundle contains the fixed-fixture compatibility mode."
fi

printf '\nAuthenticated pair\n'
if [[ "${desktop_ready}" -eq 1 && "${helper_ready}" -eq 1 ]]; then
  desktop_signature="$(codesign_field "${desktop_app}" Signature)"
  helper_signature="$(codesign_field "${installed_bundle}" Signature)"
  desktop_team="$(codesign_field "${desktop_app}" TeamIdentifier)"
  helper_team="$(codesign_field "${installed_bundle}" TeamIdentifier)"
  desktop_display="$(LC_ALL=C /usr/bin/codesign -d --verbose=4 "${desktop_app}" 2>&1 || true)"
  helper_display="$(LC_ALL=C /usr/bin/codesign -d --verbose=4 "${installed_bundle}" 2>&1 || true)"
  pair_ready=0
  expected_auth_mode=""

  if [[ "${desktop_signature}" == "adhoc" || "${helper_signature}" == "adhoc" ]]; then
    if [[ "${desktop_signature}" == "adhoc" && "${helper_signature}" == "adhoc" &&
          "${desktop_team}" == "not set" && "${helper_team}" == "not set" &&
          "${desktop_display}" == *"runtime)"* && "${helper_display}" == *"runtime)"* &&
          "${helper_authentication_mode}" == "unsafe-adhoc" &&
          "${SPICK_INPUT_ALLOW_UNSAFE_ADHOC_COMPATIBILITY:-}" == "YES" ]]; then
      pair_ready=1
      expected_auth_mode="unsafe-adhoc"
      pass "The app and helper are a statically matching unsafe ad-hoc development pair."
      note "Unsafe ad-hoc results can never qualify as supported compatibility."
    else
      issue "Ad-hoc compatibility requires a matching hardened pair and SPICK_INPUT_ALLOW_UNSAFE_ADHOC_COMPATIBILITY=YES."
    fi
  elif [[ ! "${desktop_team}" =~ ^[A-Z0-9]{10}$ ||
          "${desktop_team}" != "${helper_team}" ||
          "${helper_authentication_mode}" != "secure" ||
          "${desktop_display}" != *"runtime)"* || "${helper_display}" != *"runtime)"* ]]; then
    issue "The app and helper are not a matching hardened same-Team secure pair."
  else
    requirement_suffix="and anchor apple generic and certificate leaf[subject.OU] = \"${desktop_team}\""
    if /usr/bin/codesign --verify --deep --strict \
         -R="identifier \"${expected_desktop_identifier}\" ${requirement_suffix}" \
         "${desktop_app}" >/dev/null 2>&1 &&
       /usr/bin/codesign --verify --deep --strict \
         -R="identifier \"${expected_helper_identifier}\" ${requirement_suffix}" \
         "${installed_bundle}" >/dev/null 2>&1; then
      pair_ready=1
      expected_auth_mode="secure"
      pass "The app and helper are a matching hardened same-Team secure pair."
    else
      issue "The signed app/helper pair failed its exact Apple-Team requirements."
    fi
  fi

  if [[ "${pair_ready}" -eq 1 ]]; then
    if [[ -z "${desktop_version}" || "${desktop_version}" != "${helper_version}" ]]; then
      issue "The installed helper and desktop compatibility app versions do not match."
    else
      pass "The installed helper and desktop compatibility app versions match."
      desktop_auth="$("${desktop_executable}" --print-input-method-peer-auth-mode 2>/dev/null || true)"
      compatibility_mode="$("${desktop_executable}" --print-input-method-compatibility-mode 2>/dev/null || true)"
      input_source_state="$("${desktop_executable}" --print-input-method-compatibility-input-source-state 2>/dev/null || true)"
      if [[ "${desktop_auth}" == "${expected_auth_mode}" &&
            "${compatibility_mode}" == "fixed-fixture-v1" ]]; then
        pass "Trusted inspection commands confirm the compiled compatibility and peer-authentication modes."
      else
        issue "Trusted inspection commands reported incompatible build modes."
      fi
      if [[ "${input_source_state}" == "selected" ]]; then
        pass "TIS reports one enabled and currently selected Spick Input source."
      else
        issue "TIS does not report one enabled and currently selected Spick Input source."
      fi
    fi
  fi
else
  issue "The app/helper authentication pair could not be verified."
fi

printf '\nManual safety checks\n'
note "The runner submits only its compiled public fixture; it never records audio."
note "Never put a real secret in the password fixture."
note "Confirm Spick blocks the password field without changing it."
note "Test insertion at the beginning, middle, and end of existing text."

printf '\n'
if [[ "${readiness_issues}" -eq 0 ]]; then
  printf 'READY: all read-only preflight checks passed. No input-source state was changed.\n'
  exit 0
fi

printf 'NOT READY: %d preflight issue(s). No input-source state was changed.\n' "${readiness_issues}"
exit 1
