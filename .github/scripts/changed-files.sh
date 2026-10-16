#!/usr/bin/env bash
#
# Decide whether files relevant to a CI job changed in the current run.
#
# Usage: changed-files.sh <python|rust>
#
# Reads the standard GITHUB_* environment plus EVENT_BEFORE and DEFAULT_BRANCH
# (supplied by the calling workflow) and writes "matched=true|false" to
# $GITHUB_OUTPUT. Matching files are listed on stdout for the workflow log.
# If the change range cannot be determined, the group is reported as matched
# so that a CI job is never silently skipped.
set -euo pipefail

group="${1:-}"
case "$group" in
  python)
    patterns=(
      '*.py'
      'requirements*.txt'
      '.github/workflows/lint_python.yml'
      '.github/workflows/detect-changes.yml'
      '.github/scripts/*'
    )
    ;;
  rust)
    # The Rust workspace plus the build scripts the Rust workflow executes.
    # Files that are only copied into the .deb (Python sources, service
    # examples) do not trigger this group.
    patterns=(
      'src/rust/*'
      'src/build_dpkg.sh'
      'src/build_rust.sh'
      'src/VERSION_STRING'
      '.github/workflows/rust.yml'
      '.github/workflows/detect-changes.yml'
      '.github/scripts/*'
    )
    ;;
  *)
    echo "usage: $0 <python|rust>" >&2
    exit 2
    ;;
esac

# Report every tracked file as changed; used when no change range is available.
all_tracked_files() {
  echo "Treating every tracked file as changed." >&2
  git -c core.quotePath=false ls-files
}

# Files touched by this run, one per line.
changed_files() {
  local base_ref=""
  local merge_base

  # Tag pushes have no meaningful change range; run everything.
  if [[ "${GITHUB_REF:-}" == refs/tags/* ]]; then
    all_tracked_files
    return
  fi

  case "${GITHUB_EVENT_NAME:-}" in
    pull_request)
      # Pull requests check out a merge commit; diff against the merge base
      # so files that only changed on the base branch are not reported.
      if [[ -n "${GITHUB_BASE_REF:-}" ]]; then
        base_ref="origin/${GITHUB_BASE_REF}"
      fi
      ;;
    push)
      # github.event.before is the previously pushed commit; a new branch
      # pushes the all-zero object id and falls back to the default branch.
      if [[ -n "${EVENT_BEFORE:-}" && ! "${EVENT_BEFORE}" =~ ^0+$ ]] &&
         git rev-parse --verify --quiet "${EVENT_BEFORE}^{commit}" >/dev/null; then
        git -c core.quotePath=false diff --name-only "${EVENT_BEFORE}" "${GITHUB_SHA:-HEAD}"
        return
      fi
      if [[ -n "${DEFAULT_BRANCH:-}" ]]; then
        base_ref="origin/${DEFAULT_BRANCH}"
      fi
      ;;
    *)
      all_tracked_files
      return
      ;;
  esac

  if [[ -n "$base_ref" ]] && git rev-parse --verify --quiet "$base_ref" >/dev/null; then
    merge_base="$(git merge-base "$base_ref" HEAD)"
    git -c core.quotePath=false diff --name-only "$merge_base" HEAD
    return
  fi

  all_tracked_files
}

# Does a changed file belong to the requested group?
matches_group() {
  local pattern
  for pattern in "${patterns[@]}"; do
    if [[ "$1" == $pattern ]]; then
      return 0
    fi
  done
  return 1
}

if ! changed="$(changed_files)"; then
  changed="$(all_tracked_files)"
fi

matching_files=()
while IFS= read -r file; do
  if [[ -n "$file" ]] && matches_group "$file"; then
    matching_files+=("$file")
  fi
done <<< "$changed"

if (( ${#matching_files[@]} > 0 )); then
  echo "Change group '$group': matching files changed:"
  printf '  %s\n' "${matching_files[@]}"
else
  echo "Change group '$group': no matching files changed."
fi

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  if (( ${#matching_files[@]} > 0 )); then
    echo "matched=true" >> "$GITHUB_OUTPUT"
  else
    echo "matched=false" >> "$GITHUB_OUTPUT"
  fi
fi
