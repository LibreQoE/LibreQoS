#!/usr/bin/env bash
#
# Self-test for changed-files.sh. Run it after editing the detection logic or
# the pattern lists:
#
#   ./.github/scripts/test-changed-files.sh
#
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
detect="$script_dir/changed-files.sh"
workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

checks=0
failures=0

check() {
  local label="$1" expect="$2" group="$3"
  local out="$workdir/out.txt" log got
  rm -f "$out"
  checks=$((checks + 1))
  if ! log="$(GITHUB_OUTPUT="$out" bash "$detect" "$group" 2>&1)"; then
    echo "FAILED  [$label] $group: script exited non-zero"
    echo "$log" | sed 's/^/        | /'
    failures=$((failures + 1))
    return
  fi
  got="$(grep -E '^matched=' "$out" | cut -d= -f2)"
  if [[ "$got" == "$expect" ]]; then
    echo "ok      [$label] $group -> matched=$got"
  else
    echo "FAILED  [$label] $group: expected $expect, got '$got'"
    echo "$log" | sed 's/^/        | /'
    failures=$((failures + 1))
  fi
}

cd "$workdir"
git init -q --bare origin.git
git clone -q origin.git work
cd work
git config user.email ci@test.invalid
git config user.name "CI Test"
git symbolic-ref HEAD refs/heads/develop

mkdir -p src/rust/lqosd .github/workflows .github/scripts
printf 'base\n' > README.md
printf 'shaper\n' > src/LibreQoS.py
printf 'fn main() {}\n' > src/rust/lqosd/main.rs
printf '2.0\n' > src/VERSION_STRING
printf 'binpacking\n' > requirements.txt
printf 'pytest\n' > requirements_dev.txt
printf '# package\n' > src/build_dpkg.sh
printf '# build rust\n' > src/build_rust.sh
printf 'name: Rust\n' > .github/workflows/rust.yml
printf 'name: lint_python\n' > .github/workflows/lint_python.yml
printf 'name: detect-changes\n' > .github/workflows/detect-changes.yml
printf '# helper\n' > .github/scripts/other.sh
git add -A
git commit -qm base
git push -q origin develop

new_branch() {
  git checkout -q -b "$1" develop
}

commit_file() {
  printf '%s\n' "$2" > "$1"
  git add -A
  git commit -qm "change $1"
}

export GITHUB_REF=refs/heads/test
export GITHUB_EVENT_NAME=push
export DEFAULT_BRANCH=develop
export GITHUB_BASE_REF=""

# --- push: one file per group ---
new_branch python-change
commit_file src/LibreQoS.py '# python change'
export GITHUB_SHA="$(git rev-parse HEAD)" EVENT_BEFORE="$(git rev-parse HEAD~1)"
check "push/python" true python
check "push/python" false rust

new_branch rust-change
commit_file src/rust/lqosd/main.rs '// rust change'
export GITHUB_SHA="$(git rev-parse HEAD)" EVENT_BEFORE="$(git rev-parse HEAD~1)"
check "push/rust" true rust
check "push/rust" false python

new_branch docs-change
commit_file README.md 'more docs'
export GITHUB_SHA="$(git rev-parse HEAD)" EVENT_BEFORE="$(git rev-parse HEAD~1)"
check "push/docs" false python
check "push/docs" false rust

# --- push: pattern coverage ---
new_branch requirements-change
commit_file requirements_dev.txt 'coverage'
export GITHUB_SHA="$(git rev-parse HEAD)" EVENT_BEFORE="$(git rev-parse HEAD~1)"
check "push/requirements" true python
check "push/requirements" false rust

new_branch version-change
commit_file src/VERSION_STRING '2.1'
export GITHUB_SHA="$(git rev-parse HEAD)" EVENT_BEFORE="$(git rev-parse HEAD~1)"
check "push/version" true rust
check "push/version" false python

new_branch build-script-change
commit_file src/build_rust.sh '# tweak'
export GITHUB_SHA="$(git rev-parse HEAD)" EVENT_BEFORE="$(git rev-parse HEAD~1)"
check "push/build-script" true rust
check "push/build-script" false python

new_branch package-script-change
commit_file src/build_dpkg.sh '# tweak'
export GITHUB_SHA="$(git rev-parse HEAD)" EVENT_BEFORE="$(git rev-parse HEAD~1)"
check "push/package-script" true rust
check "push/package-script" false python

new_branch rust-workflow-change
commit_file .github/workflows/rust.yml 'name: Rust # tweak'
export GITHUB_SHA="$(git rev-parse HEAD)" EVENT_BEFORE="$(git rev-parse HEAD~1)"
check "push/rust-workflow" true rust
check "push/rust-workflow" false python

new_branch python-workflow-change
commit_file .github/workflows/lint_python.yml 'name: lint_python # tweak'
export GITHUB_SHA="$(git rev-parse HEAD)" EVENT_BEFORE="$(git rev-parse HEAD~1)"
check "push/python-workflow" true python
check "push/python-workflow" false rust

new_branch shared-workflow-change
commit_file .github/workflows/detect-changes.yml 'name: detect-changes # tweak'
export GITHUB_SHA="$(git rev-parse HEAD)" EVENT_BEFORE="$(git rev-parse HEAD~1)"
check "push/shared-workflow" true python
check "push/shared-workflow" true rust

new_branch shared-script-change
commit_file .github/scripts/other.sh '# tweak'
export GITHUB_SHA="$(git rev-parse HEAD)" EVENT_BEFORE="$(git rev-parse HEAD~1)"
check "push/shared-script" true python
check "push/shared-script" true rust

# --- push: new branch with the all-zero before ---
new_branch new-branch
commit_file src/LibreQoS.py '# new branch change'
export GITHUB_SHA="$(git rev-parse HEAD)"
export EVENT_BEFORE=0000000000000000000000000000000000000000
check "push/new-branch" true python
check "push/new-branch" false rust

# --- push: before no longer exists (force push) ---
export EVENT_BEFORE=1111111111111111111111111111111111111111
check "push/gone-before" true python
check "push/gone-before" false rust

# --- push: no before at all, falls back to the default branch ---
export EVENT_BEFORE=""
check "push/no-before" true python
check "push/no-before" false rust

# --- push: tag on an already-pushed commit ---
git checkout -q develop
export GITHUB_SHA="$(git rev-parse HEAD)"
export EVENT_BEFORE=0000000000000000000000000000000000000000
export GITHUB_REF=refs/tags/v2.0.0
check "push/tag" true python
check "push/tag" true rust
export GITHUB_REF=refs/heads/test

# --- push: non-ASCII filename ---
new_branch unicode-change
commit_file "src/café.py" '# unicode'
export GITHUB_SHA="$(git rev-parse HEAD)" EVENT_BEFORE="$(git rev-parse HEAD~1)"
check "push/unicode" true python
check "push/unicode" false rust

# --- pull_request: only the branch's own changes count ---
new_branch pr-branch
commit_file src/LibreQoS.py '# pr change'
git checkout -q develop
commit_file src/rust/lqosd/main.rs '// develop advances'
git push -q origin develop
git checkout -q pr-branch
export GITHUB_EVENT_NAME=pull_request
export GITHUB_BASE_REF=develop
export GITHUB_SHA="$(git rev-parse HEAD)"
check "pr/python-only" true python
check "pr/python-only" false rust

export GITHUB_BASE_REF=does-not-exist
check "pr/unknown-base" true python
check "pr/unknown-base" true rust

# --- unknown event: everything counts as changed ---
export GITHUB_EVENT_NAME=workflow_dispatch
export GITHUB_BASE_REF=""
check "workflow_dispatch" true python
check "workflow_dispatch" true rust

echo
if (( failures > 0 )); then
  echo "$checks checks, $failures failures"
  exit 1
fi
echo "$checks checks, 0 failures"
