#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -eq 0 ]]; then
  echo "Install Spick Input from your normal macOS account, not with sudo." >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd "${script_dir}/.." && pwd)"
source "${script_dir}/lib/input-method-signing.sh"

if [[ -n "${SPICK_INPUT_SIGNING_MODE:-}" ]]; then
  echo "Choose the installer mode with --development or --unsafe-adhoc, not SPICK_INPUT_SIGNING_MODE." >&2
  exit 1
fi
signing_mode="development"
if [[ "$#" -gt 1 ]]; then
  echo "Usage: $0 [--development|--unsafe-adhoc]" >&2
  exit 1
fi
if [[ "$#" -eq 1 ]]; then
  case "$1" in
    --development) signing_mode="development" ;;
    --unsafe-adhoc) signing_mode="unsafe-adhoc" ;;
    *)
      echo "Usage: $0 [--development|--unsafe-adhoc]" >&2
      exit 1
      ;;
  esac
fi

unsafe_build_confirmation=""
case "${signing_mode}" in
  development) ;;
  unsafe-adhoc)
    if [[ "${SPICK_INPUT_ALLOW_UNSAFE_ADHOC_INSTALL:-}" != "YES" ]]; then
      echo "Unsafe installation requires both --unsafe-adhoc and SPICK_INPUT_ALLOW_UNSAFE_ADHOC_INSTALL=YES." >&2
      exit 1
    fi
    unsafe_build_confirmation="YES"
    echo "WARNING: this ad-hoc helper can be impersonated by another process in your macOS account." >&2
    echo "Use it only for a local compatibility session; never for normal or release use." >&2
    ;;
  check)
    echo "The check artifact is intentionally non-installable." >&2
    exit 1
    ;;
  release)
    echo "Release installation must use a prebuilt, notarized payload; this development installer will not rebuild it." >&2
    exit 1
    ;;
  *)
    echo "Unknown input-method signing mode '${signing_mode}'." >&2
    exit 1
    ;;
esac

SPICK_INPUT_ALLOW_UNSAFE_ADHOC_BUILD="${unsafe_build_confirmation}" \
  spick_validate_input_signing_configuration "${signing_mode}"

build_dir="${project_dir}/target/input-method"
built_bundle="${build_dir}/Spick Input.app"
source_tool="${build_dir}/spick-input-source-tool"
input_methods_dir="${HOME}/Library/Input Methods"
installed_bundle="${input_methods_dir}/Spick Input.app"
support_dir="${HOME}/Library/Application Support/Spick"
backup_dir="${support_dir}/Input Method Backups"
lock_file="${support_dir}/input-method-install.lock"

mkdir -p "${input_methods_dir}" "${backup_dir}"
if ! /usr/bin/shlock -p "$$" -f "${lock_file}"; then
  echo "Another Spick input-method install is already running for this macOS account." >&2
  exit 1
fi

staging_dir=""
staged_bundle=""
failed_bundle=""
backup_container=""
backup_bundle=""
previous_state=""
prepared=0
installed_new_bundle=0
registration_started=0

rollback() {
  local status=$?
  local restore_failed=0
  trap - EXIT
  set +e

  if [[ "${status}" -ne 0 && "${registration_started}" -eq 0 ]]; then
    if [[ "${installed_new_bundle}" -eq 1 && -e "${installed_bundle}" ]]; then
      if ! mv "${installed_bundle}" "${failed_bundle}"; then
        echo "Could not move the failed new bundle to ${failed_bundle}." >&2
        restore_failed=1
      fi
    fi
    if [[ "${restore_failed}" -eq 0 && -n "${backup_bundle}" && -e "${backup_bundle}" ]]; then
      if ! mv "${backup_bundle}" "${installed_bundle}"; then
        echo "Could not restore the previous bundle from ${backup_bundle}." >&2
        restore_failed=1
      fi
    fi
    if [[ "${restore_failed}" -eq 0 && "${prepared}" -eq 1 ]]; then
      case "${previous_state}" in
        enabled)
          if ! "${source_tool}" register-and-enable "${installed_bundle}"; then
            restore_failed=1
          fi
          ;;
        selected)
          if ! "${source_tool}" register-and-select "${installed_bundle}"; then
            restore_failed=1
          fi
          ;;
      esac
      if [[ "${restore_failed}" -ne 0 ]]; then
        echo "The previous bundle is back, but macOS did not restore its input-source state." >&2
      fi
    fi
  elif [[ "${status}" -ne 0 ]]; then
    echo "Registration had already started, so the new bundle was left at ${installed_bundle}." >&2
    if [[ -n "${backup_bundle}" ]]; then
      echo "The previous development bundle is still available at ${backup_bundle}." >&2
    fi
  fi

  if [[ "${restore_failed}" -eq 0 ]]; then
    if [[ -n "${staging_dir}" && -d "${staging_dir}" ]]; then
      rm -rf "${staging_dir}"
    fi
    if [[ -n "${backup_container}" && -d "${backup_container}" && ! -e "${backup_bundle}" ]]; then
      rmdir "${backup_container}" 2>/dev/null || true
    fi
  elif [[ -n "${staging_dir}" ]]; then
    echo "Recovery files were preserved in ${staging_dir}." >&2
  fi
  rm -f "${lock_file}"
  exit "${status}"
}
trap rollback EXIT

staging_dir="$(mktemp -d "${support_dir}/input-method-stage.XXXXXX")"
staged_bundle="${staging_dir}/Spick Input.app"
failed_bundle="${staging_dir}/failed-Spick-Input.bundle"
build_dir="${staging_dir}/build"
built_bundle="${build_dir}/Spick Input.app"
source_tool="${build_dir}/spick-input-source-tool"
mkdir -p "${build_dir}"
SPICK_INPUT_SIGNING_MODE="${signing_mode}" \
SPICK_INPUT_ALLOW_UNSAFE_ADHOC_BUILD="${unsafe_build_confirmation}" \
SPICK_INPUT_OUTPUT_DIR="${build_dir}" \
  "${script_dir}/build-input-method.sh"
SPICK_INPUT_ALLOW_UNSAFE_ADHOC_BUILD="${unsafe_build_confirmation}" \
  spick_verify_input_artifacts "${signing_mode}" "${built_bundle}" "${source_tool}"

shopt -s nullglob
legacy_backups=("${input_methods_dir}"/Spick\ Input.backup-*.app)
shopt -u nullglob
if [[ "${#legacy_backups[@]}" -ne 0 ]]; then
  echo "Move these legacy backup bundles out of ${input_methods_dir} before installing:" >&2
  for legacy_backup in "${legacy_backups[@]}"; do
    echo "  ${legacy_backup}" >&2
  done
  exit 1
fi

previous_state="$("${source_tool}" inspect-install "${installed_bundle}")"
case "${previous_state}" in
  missing|disabled|enabled|selected) ;;
  *)
    echo "Spick Input returned an unknown installation state." >&2
    exit 1
    ;;
esac
prepared=1
"${source_tool}" prepare-install "${installed_bundle}"

ditto "${built_bundle}" "${staged_bundle}"
SPICK_INPUT_ALLOW_UNSAFE_ADHOC_BUILD="${unsafe_build_confirmation}" \
  spick_verify_input_artifacts "${signing_mode}" "${staged_bundle}" "${source_tool}"
"${source_tool}" assert-safe-to-replace "${installed_bundle}"

if [[ -e "${installed_bundle}" ]]; then
  backup_container="$(mktemp -d "${backup_dir}/backup.XXXXXX")"
  backup_bundle="${backup_container}/Spick Input.bundle-backup"
  mv "${installed_bundle}" "${backup_bundle}"
fi

mv "${staged_bundle}" "${installed_bundle}"
installed_new_bundle=1
SPICK_INPUT_ALLOW_UNSAFE_ADHOC_BUILD="${unsafe_build_confirmation}" \
  spick_verify_input_artifacts "${signing_mode}" "${installed_bundle}" "${source_tool}"
registration_started=1
"${source_tool}" register-and-select "${installed_bundle}"

trap - EXIT
rm -rf "${staging_dir}"
rm -f "${lock_file}"
echo "Installed ${installed_bundle}"
