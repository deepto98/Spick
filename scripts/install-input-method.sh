#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd "${script_dir}/.." && pwd)"
build_dir="${project_dir}/target/input-method"
built_bundle="${build_dir}/Spick Input.app"
input_methods_dir="${HOME}/Library/Input Methods"
installed_bundle="${input_methods_dir}/Spick Input.app"

"${script_dir}/build-input-method.sh"
mkdir -p "${input_methods_dir}"

if [[ -e "${installed_bundle}" ]]; then
  timestamp="$(date +%Y%m%d-%H%M%S)"
  mv "${installed_bundle}" "${input_methods_dir}/Spick Input.backup-${timestamp}.app"
fi

ditto "${built_bundle}" "${installed_bundle}"
"${build_dir}/spick-input-source-tool" register-and-select "${installed_bundle}"

echo "Installed ${installed_bundle}"
