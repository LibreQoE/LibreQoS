#!/bin/bash
#
# lqos-u5b: the v2 `repo.libreqos.com` -> v3 `packages.libreqos.com` source
# transition. Called by the `libreqos` postinst (`activate`), the v3 rollback
# (`rollback`), and removed by the postrm (`purge`). NO apt-get update here.
#
# Rules:
#   - create the v3 source only when absent; derive the suite from an existing
#     v3 source (or LQOS_REPO_SUITE, default beta for a v2->v3 box) — never
#     hardcode stable;
#   - disable (do not delete) any v2 source: `.list` -> `.v2-disabled` backup,
#     deb822 `.sources` -> a backup + `Enabled: no`;
#   - stable backup suffix, never overwrite an existing backup, never
#     double-disable;
#   - keep the v2 keyring untouched (rollback needs it);
#   - purge removes ONLY the v3 source file this helper created (marker).
set -euo pipefail

SOURCES_DIR="${LQOS_APT_SOURCES_DIR:-/etc/apt/sources.list.d}"
KEYRING="${LQOS_APT_KEYRING:-/usr/share/keyrings/libreqos-packages-keyring.gpg}"
REPO_URL="${LQOS_REPO_URL:-https://packages.libreqos.com}"
DEFAULT_SUITE="${LQOS_REPO_SUITE:-beta}"
MARKER="${LQOS_APT_MARKER:-/var/lib/libreqos/apt-source-v3.owned}"
V2_HOST="repo.libreqos.com"
V3_HOST="packages.libreqos.com"

log() { echo "libreqos-apt-source: $*" >&2; }

v3_files() {
    [ -d "$SOURCES_DIR" ] || return 0
    grep -l "$V3_HOST" "$SOURCES_DIR"/*.list "$SOURCES_DIR"/*.sources 2>/dev/null || true
}

v3_disabled_files() {
    [ -d "$SOURCES_DIR" ] || return 0
    grep -l "$V3_HOST" "$SOURCES_DIR"/*.v3-disabled 2>/dev/null || true
}

v2_disabled_files() {
    [ -d "$SOURCES_DIR" ] || return 0
    grep -l "$V2_HOST" "$SOURCES_DIR"/*.disabled "$SOURCES_DIR"/*.v2-disabled 2>/dev/null || true
}

v2_files() {
    [ -d "$SOURCES_DIR" ] || return 0
    grep -l "$V2_HOST" "$SOURCES_DIR"/*.list "$SOURCES_DIR"/*.sources 2>/dev/null || true
}

suite_from() { # suite_from FILE
    local file="$1"
    case "$file" in
        *.list*)
            sed -n "s|^[[:space:]]*deb[[:space:]].*${V3_HOST}[[:space:]]\+\([^[:space:]]*\).*|\1|p" "$file" | head -n1
            ;;
        *.sources*)
            sed -n 's/^[[:space:]]*Suites:[[:space:]]*//p' "$file" | head -n1 | awk '{print $1}'
            ;;
    esac
}

detect_suite() {
    local file suite
    while IFS= read -r file; do
        [ -n "$file" ] || continue
        suite="$(suite_from "$file")"
        if [ -n "$suite" ]; then
            printf '%s' "$suite"
            return 0
        fi
    done < <(v3_files)
    # A rollback renames the v3 source to .v3-disabled; the next activation
    # must still derive the suite (e.g. .12 stable), not fall back to beta.
    while IFS= read -r file; do
        [ -n "$file" ] || continue
        suite="$(suite_from "$file")"
        if [ -n "$suite" ]; then
            printf '%s' "$suite"
            return 0
        fi
    done < <(v3_disabled_files)
    printf '%s' "$DEFAULT_SUITE"
}

backup_name() { # backup_name FILE SUFFIX -> a non-existing stable-suffix path
    local file="$1" suffix="$2" candidate="${1}${2}" n=1
    while [ -e "$candidate" ]; do
        n=$((n + 1))
        candidate="${file}${suffix}.${n}"
    done
    printf '%s' "$candidate"
}

disable_v2() {
    local file backup
    while IFS= read -r file; do
        [ -n "$file" ] || continue
        case "$file" in
            *.sources)
                if grep -qE '^[[:space:]]*Enabled:[[:space:]]*no[[:space:]]*$' "$file"; then
                    continue
                fi
                backup="$(backup_name "$file" ".v2-disabled")"
                cp -p "$file" "$backup"
                if grep -qE '^[[:space:]]*Enabled:' "$file"; then
                    sed -i 's/^[[:space:]]*Enabled:.*/Enabled: no/' "$file"
                else
                    printf '\nEnabled: no\n' >> "$file"
                fi
                log "disabled v2 deb822 source $file (backup $backup)"
                ;;
            *.list)
                backup="$(backup_name "$file" ".v2-disabled")"
                mv "$file" "$backup"
                log "disabled v2 source $file -> $backup"
                ;;
        esac
    done < <(v2_files)
}

activate() {
    local v3 suite
    v3="$(v3_files | head -n1)"
    if [ -z "$v3" ]; then
        if [ ! -e "$KEYRING" ]; then
            log "WARNING: v3 keyring $KEYRING is absent; leaving the apt sources untouched"
            return 0
        fi
        suite="$(detect_suite)"
        mkdir -p "$SOURCES_DIR"
        # A v2 source at the v3 target name must be backed up before the v3
        # file replaces it (keep the rollback source).
        if [ -e "$SOURCES_DIR/libreqos.list" ] \
           && grep -q "$V2_HOST" "$SOURCES_DIR/libreqos.list"; then
            legacy_backup="$(backup_name "$SOURCES_DIR/libreqos.list" ".v2-disabled")"
            mv "$SOURCES_DIR/libreqos.list" "$legacy_backup"
            log "disabled v2 source $SOURCES_DIR/libreqos.list -> $legacy_backup"
        fi
        v3="$SOURCES_DIR/libreqos.list"
        printf 'deb [signed-by=%s] %s %s main\n' "$KEYRING" "$REPO_URL" "$suite" > "$v3"
        mkdir -p "$(dirname "$MARKER")"
        printf '%s\n' "$v3" > "$MARKER"
        log "added v3 source $v3 ($REPO_URL $suite main)"
    else
        suite="$(suite_from "$v3")"
        log "v3 source already present: $v3 (${suite:-unknown suite}); left untouched"
    fi
    # An already-disabled v2 (.disabled / .v2-disabled) is left untouched;
    # never re-add or overwrite it.
    while IFS= read -r file; do
        [ -n "$file" ] || continue
        log "v2 source already disabled: $file"
    done < <(v2_disabled_files)
    # Only disable v2 once a v3 channel exists.
    disable_v2
}

rollback() {
    local file backup base
    # 1. Disable/remove the v3 source FIRST, so apt cannot still see the v3
    # candidate when v2 is re-enabled.
    while IFS= read -r file; do
        [ -n "$file" ] || continue
        backup="$(backup_name "$file" ".v3-disabled")"
        mv "$file" "$backup"
        log "disabled v3 source $file -> $backup"
    done < <(v3_files)

    # 2. Re-enable the v2 sources (never the v2 keyring).
    for file in "$SOURCES_DIR"/*.disabled "$SOURCES_DIR"/*.v2-disabled "$SOURCES_DIR"/*.v2-disabled.*; do
        [ -e "$file" ] || continue
        case "$file" in
            *.v3-disabled) continue ;;
        esac
        case "$file" in
            *.disabled)
                base="${file%.disabled}"
                ;;
            *)
                base="${file%%.v2-disabled*}"
                ;;
        esac
        case "$base" in
            *.sources)
                # The deb822 file stayed in place with Enabled: no; restore the
                # pre-transition content from the backup.
                cp -p "$file" "$base"
                log "re-enabled v2 deb822 source $base from $file"
                ;;
            *)
                if [ -e "$base" ]; then
                    log "v2 source $base already present; keeping backup $file"
                    continue
                fi
                cp -p "$file" "$base"
                log "re-enabled v2 source $base from $file"
                ;;
        esac
    done
}

purge() {
    local file
    [ -f "$MARKER" ] || return 0
    while IFS= read -r file; do
        [ -n "$file" ] || continue
        case "$file" in
            "$SOURCES_DIR"/libreqos.list|"$SOURCES_DIR"/libreqos.sources)
                rm -f "$file"
                log "removed v3 source $file"
                ;;
        esac
    done < "$MARKER"
    rm -f "$MARKER"
    # The v3 keyring is shipped/owned by the control package; its postrm
    # removes it on purge. The v2 keyring is never removed here.
}

case "${1:-}" in
    activate) activate ;;
    rollback) rollback ;;
    purge) purge ;;
    *)
        echo "Usage: $0 {activate|rollback|purge}" >&2
        exit 2
        ;;
esac
