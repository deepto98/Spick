#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Spick Input can only be built on macOS." >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd "${script_dir}/.." && pwd)"
source "${script_dir}/lib/input-method-signing.sh"

signing_mode="${SPICK_INPUT_SIGNING_MODE:-check}"
spick_validate_input_signing_configuration "${signing_mode}"

source_dir="${project_dir}/macos-input-method"
default_output_dir="${project_dir}/target/input-method"
output_dir="${SPICK_INPUT_OUTPUT_DIR:-${default_output_dir}}"
lock_file="${project_dir}/target/input-method-operation.lock"
bundle_dir="${output_dir}/Spick Input.app"
contents_dir="${bundle_dir}/Contents"
executable_dir="${contents_dir}/MacOS"
resources_dir="${contents_dir}/Resources"
helper_compiler_definitions=("-DSPICK_ALLOW_UNSAFE_ADHOC_PEERS=0")
signing_identity="-"
codesign_options=("--timestamp=none")

case "${signing_mode}" in
  check) ;;
  unsafe-adhoc)
    helper_compiler_definitions=("-DSPICK_ALLOW_UNSAFE_ADHOC_PEERS=1")
    ;;
  development)
    signing_identity="${APPLE_SIGNING_IDENTITY}"
    codesign_options=("--options" "runtime" "--timestamp=none")
    ;;
  release)
    signing_identity="${APPLE_SIGNING_IDENTITY}"
    codesign_options=("--options" "runtime" "--timestamp")
    ;;
esac

mkdir -p "${project_dir}/target"
if ! /usr/bin/shlock -p "$$" -f "${lock_file}"; then
  echo "Another Spick input-method build is already running in this checkout." >&2
  exit 1
fi

release_lock() {
  local status=$?
  trap - EXIT
  rm -f "${lock_file}"
  exit "${status}"
}
trap release_lock EXIT

if [[ "${output_dir}" != "${default_output_dir}" ]]; then
  if [[ ! -d "${output_dir}" ]]; then
    echo "The private input-method build directory must already exist." >&2
    exit 1
  fi
  resolved_output_dir="$(cd "${output_dir}" && pwd -P)"
  allowed_staging_root="${HOME}/Library/Application Support/Spick"
  case "${resolved_output_dir}" in
    "${allowed_staging_root}"/input-method-stage.*/build) ;;
    *)
      echo "Private input-method builds must stay inside a Spick staging directory." >&2
      exit 1
      ;;
  esac
  output_dir="${resolved_output_dir}"
fi

if [[ -e "${bundle_dir}" ]]; then
  rm -rf "${bundle_dir}"
fi
mkdir -p "${executable_dir}" "${resources_dir}"

xcrun --sdk macosx clang \
  -fobjc-arc \
  -fmodules \
  -arch arm64 \
  -arch x86_64 \
  -Wall \
  -Wextra \
  -Werror \
  -mmacosx-version-min=13.0 \
  -framework Cocoa \
  -framework Carbon \
  -framework InputMethodKit \
  -framework Security \
  "${helper_compiler_definitions[@]}" \
  "${source_dir}/Sources/main.m" \
  "${source_dir}/Sources/SpickInputController.m" \
  "${source_dir}/Sources/SpickPeerIdentity.m" \
  "${source_dir}/Sources/SpickWireProtocol.m" \
  -o "${executable_dir}/SpickInput"

xcrun --sdk macosx clang \
  -fobjc-arc \
  -fmodules \
  -arch arm64 \
  -arch x86_64 \
  -Wall \
  -Wextra \
  -Werror \
  -mmacosx-version-min=13.0 \
  -framework Cocoa \
  -framework Carbon \
  "${source_dir}/Sources/SpickInputSourceTool.m" \
  -o "${output_dir}/spick-input-source-tool"

cp "${source_dir}/Info.plist" "${contents_dir}/Info.plist"
cp "${project_dir}/src-tauri/icons/icon.icns" "${resources_dir}/SpickInput.icns"
build_number="${SPICK_INPUT_BUILD_NUMBER:-$(git -C "${project_dir}" rev-list --count HEAD).$(date +%s)}"
if [[ ! "${build_number}" =~ ^[1-9][0-9]*(\.[0-9]+){0,2}$ ]]; then
  echo "SPICK_INPUT_BUILD_NUMBER must be one to three numeric components and start above zero." >&2
  exit 1
fi
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion ${build_number}" "${contents_dir}/Info.plist"
plutil -lint "${contents_dir}/Info.plist" >/dev/null
/usr/bin/codesign --force --sign "${signing_identity}" \
  --identifier "${spick_input_helper_identifier}" \
  "${codesign_options[@]}" "${bundle_dir}" >/dev/null
/usr/bin/codesign --force --sign "${signing_identity}" \
  --identifier "${spick_input_tool_identifier}" \
  "${codesign_options[@]}" "${output_dir}/spick-input-source-tool" >/dev/null
spick_verify_input_artifacts "${signing_mode}" "${bundle_dir}" \
  "${output_dir}/spick-input-source-tool"
"${executable_dir}/SpickInput" --protocol-self-test
"${executable_dir}/SpickInput" --peer-auth-runtime-self-test
lipo "${executable_dir}/SpickInput" -verify_arch arm64 x86_64
lipo "${output_dir}/spick-input-source-tool" -verify_arch arm64 x86_64

echo "Built ${bundle_dir} (${signing_mode})"
