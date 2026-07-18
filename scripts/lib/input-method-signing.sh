#!/usr/bin/env bash

# Shared fail-closed policy for building and installing Spick Input. Callers
# must enable `set -euo pipefail` before sourcing this file.

spick_input_helper_identifier="app.spick.desktop.input-method"
spick_input_tool_identifier="app.spick.desktop.input-source-tool"

spick_input_signing_fail() {
  echo "$1" >&2
  return 1
}

spick_validate_input_signing_configuration() {
  local mode="$1"
  local identity="${APPLE_SIGNING_IDENTITY:-}"
  local team_id="${SPICK_INPUT_TEAM_ID:-}"
  local unsafe_build="${SPICK_INPUT_ALLOW_UNSAFE_ADHOC_BUILD:-}"

  case "${mode}" in
    check)
      if [[ -n "${identity}" || -n "${team_id}" || -n "${unsafe_build}" ]]; then
        spick_input_signing_fail \
          "The check build ignores signing credentials; unset APPLE_SIGNING_IDENTITY, SPICK_INPUT_TEAM_ID, and unsafe flags."
      fi
      ;;
    unsafe-adhoc)
      if [[ "${unsafe_build}" != "YES" ]]; then
        spick_input_signing_fail \
          "Unsafe ad-hoc builds require SPICK_INPUT_ALLOW_UNSAFE_ADHOC_BUILD=YES."
      fi
      if [[ -n "${identity}" || -n "${team_id}" ]]; then
        spick_input_signing_fail \
          "Unsafe ad-hoc builds cannot use APPLE_SIGNING_IDENTITY or SPICK_INPUT_TEAM_ID."
      fi
      ;;
    development|release)
      if [[ -n "${unsafe_build}" ]]; then
        spick_input_signing_fail \
          "Signed builds refuse SPICK_INPUT_ALLOW_UNSAFE_ADHOC_BUILD."
      fi
      if [[ ! "${identity}" =~ ^[0-9A-Fa-f]{40}$ ]]; then
        spick_input_signing_fail \
          "${mode} builds require APPLE_SIGNING_IDENTITY as a 40-character certificate SHA-1 hash."
      fi
      if [[ ! "${team_id}" =~ ^[A-Z0-9]{10}$ ]]; then
        spick_input_signing_fail \
          "${mode} builds require a 10-character SPICK_INPUT_TEAM_ID."
      fi

      local identity_line
      identity_line="$(/usr/bin/security find-identity -v -p codesigning 2>/dev/null | \
        /usr/bin/awk -v wanted="${identity}" \
          'toupper($2) == toupper(wanted) { print; exit }')"
      if [[ -z "${identity_line}" ]]; then
        spick_input_signing_fail \
          "APPLE_SIGNING_IDENTITY is not a valid code-signing identity in this keychain."
      fi
      if [[ "${mode}" == "development" && "${identity_line}" != *'"Apple Development:'* ]]; then
        spick_input_signing_fail \
          "Development builds require an Apple Development identity."
      fi
      if [[ "${mode}" == "release" && "${identity_line}" != *'"Developer ID Application:'* ]]; then
        spick_input_signing_fail \
          "Release builds require a Developer ID Application identity."
      fi
      ;;
    *)
      spick_input_signing_fail \
        "Unknown SPICK_INPUT_SIGNING_MODE '${mode}'; use check, development, release, or unsafe-adhoc."
      ;;
  esac
}

spick_codesign_display() {
  LC_ALL=C /usr/bin/codesign -d --verbose=4 "$1" 2>&1
}

spick_codesign_value() {
  local display="$1"
  local key="$2"
  /usr/bin/sed -n "s/^${key}=//p" <<<"${display}" | /usr/bin/head -n 1
}

spick_verify_input_artifact_shape() {
  local bundle="$1"
  local tool="$2"
  local helper="${bundle}/Contents/MacOS/SpickInput"
  local current_user
  current_user="$(/usr/bin/id -u)"

  if [[ ! -d "${bundle}" || -L "${bundle}" || ! -O "${bundle}" ]]; then
    spick_input_signing_fail "Spick Input must be a user-owned bundle, not a symlink."
  fi
  if [[ ! -f "${helper}" || ! -x "${helper}" || -L "${helper}" || ! -O "${helper}" ]]; then
    spick_input_signing_fail "Spick Input's executable has an unsafe file shape."
  fi
  if [[ ! -f "${tool}" || ! -x "${tool}" || -L "${tool}" || ! -O "${tool}" ]]; then
    spick_input_signing_fail "The Spick input-source tool has an unsafe file shape."
  fi
  if [[ "$(/usr/bin/stat -f '%u' "${bundle}")" != "${current_user}" ||
        "$(/usr/bin/stat -f '%u' "${helper}")" != "${current_user}" ||
        "$(/usr/bin/stat -f '%u' "${tool}")" != "${current_user}" ]]; then
    spick_input_signing_fail "Spick Input artifacts must belong to the current user."
  fi
}

spick_verify_input_artifacts() {
  local mode="$1"
  local bundle="$2"
  local tool="$3"
  local helper="${bundle}/Contents/MacOS/SpickInput"

  spick_validate_input_signing_configuration "${mode}"
  spick_verify_input_artifact_shape "${bundle}" "${tool}"
  /usr/bin/codesign --verify --deep --strict "${bundle}"
  /usr/bin/codesign --verify --strict "${tool}"

  local helper_display tool_display helper_id tool_id helper_team tool_team
  helper_display="$(spick_codesign_display "${bundle}")"
  tool_display="$(spick_codesign_display "${tool}")"
  helper_id="$(spick_codesign_value "${helper_display}" "Identifier")"
  tool_id="$(spick_codesign_value "${tool_display}" "Identifier")"
  helper_team="$(spick_codesign_value "${helper_display}" "TeamIdentifier")"
  tool_team="$(spick_codesign_value "${tool_display}" "TeamIdentifier")"

  if [[ "${helper_id}" != "${spick_input_helper_identifier}" ||
        "${tool_id}" != "${spick_input_tool_identifier}" ]]; then
    spick_input_signing_fail "Spick Input has an unexpected code-signing identifier."
  fi

  if [[ "${mode}" == "check" || "${mode}" == "unsafe-adhoc" ]]; then
    if [[ "$(spick_codesign_value "${helper_display}" "Signature")" != "adhoc" ||
          "$(spick_codesign_value "${tool_display}" "Signature")" != "adhoc" ||
          "${helper_team}" != "not set" || "${tool_team}" != "not set" ]]; then
      spick_input_signing_fail "Ad-hoc modes require two ad-hoc artifacts with no Team ID."
    fi
    /usr/bin/codesign --verify --deep --strict \
      -R="identifier \"${spick_input_helper_identifier}\"" "${bundle}"
    /usr/bin/codesign --verify --strict \
      -R="identifier \"${spick_input_tool_identifier}\"" "${tool}"
    if /usr/bin/codesign --verify --strict -R='anchor apple generic' \
      "${bundle}" >/dev/null 2>&1; then
      spick_input_signing_fail "An ad-hoc build unexpectedly satisfied an Apple trust anchor."
    fi
  else
    local team_id="${SPICK_INPUT_TEAM_ID}"
    if [[ "${helper_team}" != "${team_id}" || "${tool_team}" != "${team_id}" ||
          "$(spick_codesign_value "${helper_display}" "Signature")" == "adhoc" ||
          "$(spick_codesign_value "${tool_display}" "Signature")" == "adhoc" ]]; then
      spick_input_signing_fail "Spick Input is not signed by the configured Apple Developer Team."
    fi

    local helper_requirement tool_requirement
    helper_requirement="identifier \"${spick_input_helper_identifier}\" and anchor apple generic and certificate leaf[subject.OU] = \"${team_id}\""
    tool_requirement="identifier \"${spick_input_tool_identifier}\" and anchor apple generic and certificate leaf[subject.OU] = \"${team_id}\""
    if [[ "${mode}" == "release" ]]; then
      local developer_id_requirement=' and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists'
      helper_requirement+="${developer_id_requirement}"
      tool_requirement+="${developer_id_requirement}"
    fi
    /usr/bin/codesign --verify --deep --strict -R="${helper_requirement}" "${bundle}"
    /usr/bin/codesign --verify --strict -R="${tool_requirement}" "${tool}"

    if [[ "${helper_display}" != *"runtime)"* ||
          "${tool_display}" != *"runtime)"* ]]; then
      spick_input_signing_fail \
        "Signed Spick Input artifacts require hardened runtime."
    fi
    if [[ "${mode}" == "release" ]]; then
      if [[ "${helper_display}" != *$'\nTimestamp='* ||
            "${tool_display}" != *$'\nTimestamp='* ]]; then
        spick_input_signing_fail \
          "Release artifacts require hardened runtime and a trusted timestamp."
      fi
    fi
  fi

  local expected_auth_mode="secure"
  if [[ "${mode}" == "unsafe-adhoc" ]]; then
    expected_auth_mode="unsafe-adhoc"
  fi
  local actual_auth_mode
  actual_auth_mode="$("${helper}" --print-peer-auth-mode)"
  if [[ "${actual_auth_mode}" != "${expected_auth_mode}" ]]; then
    spick_input_signing_fail \
      "Spick Input's compiled peer-authentication mode does not match its signing mode."
  fi
}
