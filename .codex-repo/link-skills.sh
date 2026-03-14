#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_SKILLS_DIR="${SCRIPT_DIR}/skills"
CODEX_HOME_DIR="${CODEX_HOME:-$HOME/.codex}"
USER_SKILLS_DIR="${CODEX_HOME_DIR}/skills"

mkdir -p "${USER_SKILLS_DIR}"

if [ ! -d "${REPO_SKILLS_DIR}" ]; then
    echo "Repo skills directory not found: ${REPO_SKILLS_DIR}" >&2
    exit 1
fi

linked_any=0

for skill_dir in "${REPO_SKILLS_DIR}"/*; do
    [ -d "${skill_dir}" ] || continue
    [ -f "${skill_dir}/SKILL.md" ] || continue

    skill_name="$(basename "${skill_dir}")"
    target="${USER_SKILLS_DIR}/${skill_name}"

    if [ -L "${target}" ]; then
        current_target="$(readlink -f "${target}")"
        desired_target="$(readlink -f "${skill_dir}")"
        if [ "${current_target}" = "${desired_target}" ]; then
            echo "Already linked: ${skill_name}"
            continue
        fi
        echo "Skipping ${skill_name}: ${target} already points to ${current_target}" >&2
        continue
    fi

    if [ -e "${target}" ]; then
        echo "Skipping ${skill_name}: ${target} already exists and is not a symlink" >&2
        continue
    fi

    ln -s "${skill_dir}" "${target}"
    echo "Linked ${skill_name} -> ${skill_dir}"
    linked_any=1
done

if [ "${linked_any}" -eq 0 ]; then
    echo "No new skills linked."
fi

echo "Restart Codex to pick up skill changes."
