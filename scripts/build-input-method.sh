#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Spick Input can only be built on macOS." >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd "${script_dir}/.." && pwd)"
source_dir="${project_dir}/macos-input-method"
output_dir="${project_dir}/target/input-method"
bundle_dir="${output_dir}/Spick Input.app"
contents_dir="${bundle_dir}/Contents"
executable_dir="${contents_dir}/MacOS"
resources_dir="${contents_dir}/Resources"

if [[ -e "${bundle_dir}" ]]; then
  rm -rf "${bundle_dir}"
fi
mkdir -p "${executable_dir}" "${resources_dir}"

xcrun --sdk macosx clang \
  -fobjc-arc \
  -fmodules \
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
plutil -lint "${contents_dir}/Info.plist" >/dev/null
"${executable_dir}/SpickInput" --protocol-self-test
codesign --force --deep --sign - "${bundle_dir}" >/dev/null

echo "Built ${bundle_dir}"
