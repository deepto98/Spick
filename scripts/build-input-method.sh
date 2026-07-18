#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Spick Input can only be built on macOS." >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd "${script_dir}/.." && pwd)"
source_dir="${project_dir}/macos-input-method"
default_output_dir="${project_dir}/target/input-method"
output_dir="${SPICK_INPUT_OUTPUT_DIR:-${default_output_dir}}"
lock_file="${project_dir}/target/input-method-operation.lock"
bundle_dir="${output_dir}/Spick Input.app"
contents_dir="${bundle_dir}/Contents"
executable_dir="${contents_dir}/MacOS"
resources_dir="${contents_dir}/Resources"

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
  "${source_dir}/Sources/main.m" \
  "${source_dir}/Sources/SpickInputController.m" \
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
"${executable_dir}/SpickInput" --protocol-self-test
codesign --force --deep --sign - "${bundle_dir}" >/dev/null
codesign --force --sign - "${output_dir}/spick-input-source-tool" >/dev/null
lipo "${executable_dir}/SpickInput" -verify_arch arm64 x86_64
lipo "${output_dir}/spick-input-source-tool" -verify_arch arm64 x86_64
codesign --verify --deep --strict "${bundle_dir}"
codesign --verify --strict "${output_dir}/spick-input-source-tool"

echo "Built ${bundle_dir}"
