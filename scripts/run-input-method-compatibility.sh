#!/usr/bin/env bash
set -euo pipefail

# This runner may launch one fixed-fixture probe and write private evidence. It
# never builds, installs, registers, enables, disables, selects, or deselects an
# input source, and it never opens or focuses a target application.

if [[ "$#" -ne 4 || "$1" != "--case" || -z "$2" ||
      "$3" != "--profile" || ( "$4" != "cold" && "$4" != "warm" ) ]]; then
  echo "Usage: $0 --case <catalog-case-id> --profile cold|warm" >&2
  exit 64
fi
if [[ ! -t 0 || ! -t 1 ]]; then
  echo "The compatibility runner requires an interactive terminal." >&2
  exit 64
fi
if [[ "$(/usr/bin/id -u)" -eq 0 ]]; then
  echo "Do not run the compatibility harness with sudo." >&2
  exit 1
fi
if [[ "$(/usr/bin/uname -s)" != "Darwin" ]]; then
  echo "Spick input-method compatibility runs require macOS." >&2
  exit 1
fi

case_id="$2"
run_profile="$4"
script_dir="$(cd "$(/usr/bin/dirname "${BASH_SOURCE[0]}")" && pwd -P)"
project_dir="$(cd "${script_dir}/.." && pwd -P)"
app="${project_dir}/src-tauri/target/debug/bundle/macos/Spick.app"
executable="${app}/Contents/MacOS/spick-desktop"
app_info="${app}/Contents/Info.plist"
evidence_dir="${project_dir}/target/input-method-compat"
attempt_path=""
probe_pid=""
watchdog_pid=""

cleanup_probe() {
  if [[ -n "${probe_pid}" ]] && /bin/kill -0 "${probe_pid}" 2>/dev/null; then
    /bin/kill -TERM "${probe_pid}" 2>/dev/null || true
    /bin/sleep 2
    if /bin/kill -0 "${probe_pid}" 2>/dev/null; then
      /bin/kill -KILL "${probe_pid}" 2>/dev/null || true
    fi
    wait "${probe_pid}" 2>/dev/null || true
  fi
  if [[ -n "${watchdog_pid}" ]] && /bin/kill -0 "${watchdog_pid}" 2>/dev/null; then
    /bin/kill -TERM "${watchdog_pid}" 2>/dev/null || true
    wait "${watchdog_pid}" 2>/dev/null || true
  fi
}
interrupt_probe() {
  cleanup_probe
  trap - HUP INT TERM EXIT
  exit 130
}
trap interrupt_probe HUP INT TERM
trap cleanup_probe EXIT

"${script_dir}/preflight-input-method-compatibility.sh"

if [[ ! -f "${executable}" || ! -x "${executable}" || -L "${executable}" ||
      ! -O "${executable}" ]]; then
  echo "No safe compatibility executable is available at ${executable}." >&2
  exit 1
fi
if /usr/bin/pgrep -x spick-desktop >/dev/null 2>&1; then
  echo "Quit every running Spick process before starting a one-shot compatibility run." >&2
  exit 1
fi
if [[ ! -f "${app_info}" || -L "${app_info}" ||
      "$(/usr/libexec/PlistBuddy -c 'Print :SpickInputCompatibilityMode' "${app_info}" 2>/dev/null || true)" != "fixed-fixture-v1" ]]; then
  echo "The signed desktop bundle does not contain the fixed-fixture compatibility marker." >&2
  exit 1
fi
if [[ "$("${executable}" --print-input-method-compatibility-mode)" != "fixed-fixture-v1" ]]; then
  echo "The desktop bundle was not built in fixed-fixture compatibility mode." >&2
  exit 1
fi

case_description="$("${executable}" --describe-input-method-compatibility-case "${case_id}")"
printf '\n%s\n\n' "${case_description}"
printf 'This run uses only the fixture printed above. It will not use the microphone, Whisper, the clipboard, or your Spick settings.\n'
printf 'Prepare a disposable target exactly as described. Return here without focusing it yet.\n'
if [[ "${run_profile}" == "cold" ]]; then
  printf 'Cold profile: fully quit and reopen the target app; this must be the first compatibility attempt in that app launch.\n'
  profile_confirmation="COLD"
else
  printf 'Warm profile: keep the same target-app launch after a completed matching attempt, but reset the disposable field/document.\n'
  profile_confirmation="WARM"
fi
read -r -p "Type ${profile_confirmation} to attest that profile and arm this one case: " confirmation
if [[ "${confirmation}" != "${profile_confirmation}" ]]; then
  echo "Compatibility run cancelled; no probe was launched."
  exit 0
fi

umask 077
if [[ -e "${evidence_dir}" ]]; then
  if [[ ! -d "${evidence_dir}" || -L "${evidence_dir}" || ! -O "${evidence_dir}" ]]; then
    echo "The compatibility evidence path has an unsafe shape." >&2
    exit 1
  fi
else
  /bin/mkdir -p "${evidence_dir}"
fi
/bin/chmod 700 "${evidence_dir}"
if [[ "$(/usr/bin/stat -f '%OLp' "${evidence_dir}")" != "700" ]]; then
  echo "The compatibility evidence directory is not private." >&2
  exit 1
fi

run_id="$(/usr/bin/uuidgen | /usr/bin/tr '[:upper:]' '[:lower:]')"
attempt_path="${evidence_dir}/attempt-${run_id}.json"

printf '\nThe probe process is starting. Wait for its "Compatibility case ... is ready" line, then focus the prepared target and press and release Command/Control + Shift + Space once.\n'
set +e
"${executable}" --input-method-compatibility-probe \
  "${case_id}" "${run_id}" "${run_profile}" &
probe_pid=$!
(
  /bin/sleep 55
  if /bin/kill -0 "${probe_pid}" 2>/dev/null; then
    /bin/kill -TERM "${probe_pid}" 2>/dev/null || true
    /bin/sleep 2
    if /bin/kill -0 "${probe_pid}" 2>/dev/null; then
      /bin/kill -KILL "${probe_pid}" 2>/dev/null || true
    fi
  fi
) &
watchdog_pid=$!
wait "${probe_pid}"
probe_status=$?
probe_pid=""
if /bin/kill -0 "${watchdog_pid}" 2>/dev/null; then
  /bin/kill -TERM "${watchdog_pid}" 2>/dev/null || true
fi
wait "${watchdog_pid}" 2>/dev/null || true
watchdog_pid=""
set -e

if [[ ! -f "${attempt_path}" || -L "${attempt_path}" || ! -O "${attempt_path}" ]]; then
  echo "The probe exited with status ${probe_status}, but no safe attempt record was produced." >&2
  exit 1
fi
if [[ "$(/usr/bin/stat -f '%OLp' "${attempt_path}")" != "400" ]]; then
  echo "The attempt record was not sealed read-only." >&2
  exit 1
fi
technical_verdict="$(/usr/bin/plutil -extract observation.verdict raw -o - "${attempt_path}")"
expected_observation="$(/usr/bin/plutil -extract expectedObservation raw -o - "${attempt_path}")"
recorded_case="$(/usr/bin/plutil -extract caseId raw -o - "${attempt_path}")"
recorded_profile="$(/usr/bin/plutil -extract runProfile raw -o - "${attempt_path}")"
if [[ "${recorded_case}" != "${case_id}" || "${recorded_profile}" != "${run_profile}" ]]; then
  echo "The attempt record does not match the requested case/profile." >&2
  exit 1
fi
lifecycle_outcome="nonzeroExit"
if [[ "${probe_status}" -eq 0 ]]; then
  lifecycle_outcome="cleanExit"
fi
"${executable}" --record-input-method-compatibility-lifecycle \
  "${run_id}" "${lifecycle_outcome}"
printf '\nMachine observation: %s (process status %d)\n' "${technical_verdict}" "${probe_status}"

printf '\nLook at the disposable target now. Does it exactly match this case’s expected document state?\n'
printf '  1  Yes, exact\n  2  No, unexpected\n  3  I cannot verify\n'
read -r -p 'Document result [1-3]: ' document_choice
case "${document_choice}" in
  1) document_observation="exactCatalogExpected" ;;
  2) document_observation="unexpected" ;;
  3) document_observation="notVerifiable" ;;
  *) echo "No review was written because the document choice was invalid." >&2; exit 64 ;;
esac

if [[ "${expected_observation}" == "confirmedInsertion" ]]; then
  printf '\nDid the caret/selection finish exactly where this case expects?\n'
  printf '  1  Yes\n  2  No\n  3  I cannot verify\n'
  read -r -p 'Caret result [1-3]: ' caret_choice
  case "${caret_choice}" in
    1) caret_observation="expected" ;;
    2) caret_observation="unexpected" ;;
    3) caret_observation="notVerifiable" ;;
    *) echo "No review was written because the caret choice was invalid." >&2; exit 64 ;;
  esac
else
  caret_observation="notApplicable"
fi

external_observation="notApplicable"
if [[ "${case_id}" == macos.terminal.* || "${case_id}" == macos.slack.* ]]; then
  printf '\nWas anything sent or executed?\n'
  printf '  1  Nothing was sent or executed\n  2  A message was sent\n  3  A command executed\n  4  I cannot verify\n'
  read -r -p 'External-action result [1-4]: ' external_choice
  case "${external_choice}" in
    1) external_observation="noneObserved" ;;
    2) external_observation="messageSent" ;;
    3) external_observation="commandExecuted" ;;
    4) external_observation="notVerifiable" ;;
    *) echo "No review was written because the external-action choice was invalid." >&2; exit 64 ;;
  esac
fi

review_id="$(/usr/bin/uuidgen | /usr/bin/tr '[:upper:]' '[:lower:]')"
"${executable}" --record-input-method-compatibility-review \
  "${run_id}" \
  "${review_id}" \
  "${document_observation}" \
  "${caret_observation}" \
  "${external_observation}"

classification="$(
  "${executable}" --classify-input-method-compatibility-attempt "${run_id}"
)"

trap - HUP INT TERM EXIT
printf '\nRecorded immutable attempt and manual-review evidence under %s.\n' "${evidence_dir}"
if [[ "${probe_status}" -ne 0 ]]; then
  echo "RESULT: the probe process did not complete cleanly; this attempt cannot qualify."
  exit 1
fi
case "${classification}" in
  qualifyingPass)
    echo "RESULT: this is one qualifying reviewed pass. Three matching signed attempts are required before claiming support."
    ;;
  nonQualifyingMatch)
    echo "RESULT: the observation matched, but dirty, unsafe, or incomplete provenance makes it non-qualifying."
    exit 2
    ;;
  unsafe)
    echo "RESULT: an unexpected document or external action makes this attempt unsafe."
    exit 1
    ;;
  fail)
    echo "RESULT: this reviewed attempt did not pass."
    exit 1
    ;;
  *)
    echo "The compatibility classifier returned an unknown result: ${classification}" >&2
    exit 1
    ;;
esac
