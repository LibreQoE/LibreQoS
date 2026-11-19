//! Security card 94b00da77: the mandatory one-time first-run setup CODE.
//!
//! The code is generated ONLY at startup/reset while the instance is
//! unconfigured (CSPRNG, never on the request path, BEFORE the listener
//! binds), stored ROOT-ONLY under /run/lqos/first_run (dir 0700, file 0600,
//! O_EXCL create, O_NOFOLLOW read), verified in CONSTANT TIME, and DURABLY
//! consumed BEFORE the first Admin create (P7). The caller serializes with an
//! IN-PROCESS mutex (P13) — never a second lock file. The code is NEVER
//! logged — only its PATH, at a visible level, with the `sudo cat`
//! instruction.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The dedicated root-only directory (0700) under the shared /run/lqos.
pub(crate) const CODE_DIR: &str = "/run/lqos/first_run";
/// The code file name (0600).
pub(crate) const CODE_FILE: &str = "setup-code";
const CODE_BYTES: usize = 32;
/// A generated code is 32 random bytes hex-encoded: a FIXED length. A stored
/// file of any other length is refused (a short file must never verify short).
const CODE_HEX_LEN: usize = CODE_BYTES * 2;

/// The startup/reset generation failed to establish a usable code. Set by
/// `ensure_first_run_setup_code`; the request path refuses (503) while set, so a
/// stale file surviving a failed remove/create is never accepted. Fail closed.
static CODE_UNAVAILABLE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Test-only override: the code directory. Production always uses `CODE_DIR`.
#[cfg(test)]
static TEST_CODE_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
/// Test-only override: the required owner uid (tests run unprivileged).
#[cfg(test)]
static TEST_REQUIRED_UID: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(u32::MAX);

fn code_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(dir) = TEST_CODE_DIR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
    {
        return dir;
    }
    PathBuf::from(CODE_DIR)
}

/// The code file path (printed by the boot log — never the code).
pub(crate) fn code_path() -> PathBuf {
    code_dir().join(CODE_FILE)
}

/// The required owner uid for the code dir + file. Root in production; tests
/// inject the current uid so the owner/mode checks are exercised unprivileged.
fn required_uid() -> u32 {
    #[cfg(test)]
    {
        let uid = TEST_REQUIRED_UID.load(std::sync::atomic::Ordering::Relaxed);
        if uid != u32::MAX {
            return uid;
        }
    }
    0
}

/// True when startup could not establish a code; the handler must refuse.
pub(crate) fn is_unavailable() -> bool {
    CODE_UNAVAILABLE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Startup established a code: the request path may verify it.
pub(crate) fn mark_available() {
    CODE_UNAVAILABLE.store(false, std::sync::atomic::Ordering::Relaxed);
}

/// Startup could not establish a code: refuse every first-run request.
pub(crate) fn mark_unavailable() {
    CODE_UNAVAILABLE.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Writes the code with mode 0600 and O_NOFOLLOW (never follows a symlink).
/// O_EXCL: a live code file is never silently overwritten; a stale one is
/// removed explicitly by `generate_unconfigured`.
fn write_code_file(path: &Path, code: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    file.write_all(code.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    file.sync_all()
        .map_err(|e| format!("sync {}: {e}", path.display()))?;
    // Explicit mode even if the file pre-existed.
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("chmod {}: {e}", path.display()))?;
    Ok(())
}

/// Constant-time byte comparison (no early exit; length is public).
fn ct_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// Generates a fresh code and persists it ROOT-ONLY. Startup/reset only;
/// fail-closed (an unwritable file is an explicit error — the first-run flow
/// then refuses rather than exposing an unguarded endpoint).
pub(crate) fn generate_unconfigured() -> Result<PathBuf, String> {
    generate_in_dir(&code_dir())
}

fn generate_in_dir(dir: &Path) -> Result<PathBuf, String> {
    // The shared /run/lqos is used by other components (bus socket, lqosd
    // lock): never force it 0700. Only the dedicated first_run subdir is
    // root-only.
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("create {}: {e}", dir.display()))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("chmod {}: {e}", dir.display()))?;

    // Generation is startup/reset only and single-writer: a stale code from a
    // crash is removed explicitly (never blindly truncated). A failed remove is
    // an error: the caller latches the first-run path closed, so a stale code
    // is never accepted.
    let path = dir.join(CODE_FILE);
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("remove stale {}: {e}", path.display())),
    }

    let mut entropy = [0u8; CODE_BYTES];
    let mut urandom =
        fs::File::open("/dev/urandom").map_err(|e| format!("open /dev/urandom: {e}"))?;
    urandom
        .read_exact(&mut entropy)
        .map_err(|e| format!("read /dev/urandom: {e}"))?;

    write_code_file(&path, &hex_encode(&entropy))?;
    Ok(path)
}

fn read_code_at(path: &Path, required_uid: u32) -> Result<String, String> {
    // The directory is root-only too: a symlinked/replaced directory with the
    // right file mode must not admit a code planted elsewhere.
    let Some(dir) = path.parent() else {
        return Err("code path has no parent directory".to_string());
    };
    let dir_metadata = fs::metadata(dir)
        .map_err(|e| format!("stat dir {}: {e}", dir.display()))?;
    if !dir_metadata.is_dir() {
        return Err("code parent is not a directory".to_string());
    }
    if dir_metadata.uid() != required_uid {
        return Err(format!("wrong dir owner uid={}", dir_metadata.uid()));
    }
    let dir_mode = dir_metadata.mode() & 0o777;
    if dir_mode != 0o700 {
        return Err(format!("wrong dir mode {dir_mode:o}"));
    }

    // O_NOFOLLOW refuses a symlink at the final component.
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("stat {}: {e}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err("not a regular file".to_string());
    }
    if metadata.uid() != required_uid {
        return Err(format!("wrong owner uid={}", metadata.uid()));
    }
    let mode = metadata.mode() & 0o777;
    if mode != 0o600 {
        return Err(format!("wrong mode {mode:o}"));
    }
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("empty setup code".to_string());
    }
    // Fixed length: a short/truncated root-only file must never verify short.
    if trimmed.len() != CODE_HEX_LEN {
        return Err(format!(
            "malformed setup code length {} (expected {CODE_HEX_LEN})",
            trimmed.len()
        ));
    }
    Ok(trimmed.to_string())
}

/// Verifies a candidate code in constant time against the stored code.
/// Wrong/missing/unreadable => false.
pub(crate) fn verify(candidate: &str) -> bool {
    verify_at(candidate, &code_path(), required_uid())
}

fn verify_at(candidate: &str, path: &Path, required_uid: u32) -> bool {
    let Ok(stored) = read_code_at(path, required_uid) else {
        return false;
    };
    ct_eq(stored.as_bytes(), candidate.trim().as_bytes())
}

/// DURABLY consumes the code (unlink + fsync the parent dir). Called BEFORE
/// the Admin create (P7): a crash after this leaves NO live code and NO admin;
/// a restart of the still-unconfigured instance generates a fresh code.
/// Consuming after the create could re-arm the code on a crash.
pub(crate) fn consume_durable() -> Result<(), String> {
    consume_durable_at(&code_path())
}

fn consume_durable_at(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("unlink {}: {e}", path.display())),
    }
    // Make the unlink itself durable before the create is attempted.
    if let Some(parent) = path.parent() {
        let dir = fs::File::open(parent)
            .map_err(|e| format!("open dir {}: {e}", parent.display()))?;
        dir.sync_all()
            .map_err(|e| format!("fsync dir {}: {e}", parent.display()))?;
    }
    Ok(())
}

/// Consumes the code (unlink). Kept for the Ready-purge call site; the
/// request path MUST use `consume_durable` so the ordering is durable.
pub(crate) fn consume() -> Result<(), String> {
    consume_durable()
}

// ---- test-only injection points (never compiled into production) ----

#[cfg(test)]
pub(crate) fn set_test_code_dir(dir: Option<PathBuf>) {
    *TEST_CODE_DIR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = dir;
}

#[cfg(test)]
pub(crate) fn set_test_required_uid(uid: u32) {
    TEST_REQUIRED_UID.store(uid, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
pub(crate) fn reset_latch_for_test() {
    CODE_UNAVAILABLE.store(false, std::sync::atomic::Ordering::Relaxed);
}

/// Writes a KNOWN code into `dir` with the production modes (dir 0700, file
/// 0600), so a handler test can present a correct code without guessing the
/// CSPRNG output.
#[cfg(test)]
pub(crate) fn write_code_for_test(dir: &Path, code: &str) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("chmod {}: {e}", dir.display()))?;
    let path = dir.join(CODE_FILE);
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("remove {}: {e}", path.display())),
    }
    write_code_file(&path, code)
}

/// The visible boot line: the PATH + the sudo-cat instruction. NEVER the code.
pub(crate) fn log_location() {
    tracing::warn!(
        "First-run setup code is at {} (root-only). Read it with: sudo cat {} — never logged.",
        code_path().display(),
        code_path().display(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 32 bytes hex-encoded — the fixed production length.
    const CODE_A: &str = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    const CODE_B: &str = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeee";

    #[test]
    fn constant_time_compare_matches_and_rejects() {
        assert!(ct_eq(b"abc123", b"abc123"));
        assert!(!ct_eq(b"abc123", b"abc124"));
        assert!(!ct_eq(b"abc123", b"abc12"));
        assert!(!ct_eq(b"", b"a"));
        assert!(ct_eq(b"", b""));
    }

    #[test]
    fn the_code_path_is_under_run() {
        // Assert the PRODUCTION constants, not code_path(): the cfg(test)
        // override may be held by a parallel handler test with a temp dir, so
        // reading it here is a race (not a code-path defect).
        assert_eq!(CODE_DIR, "/run/lqos/first_run");
        assert_eq!(CODE_FILE, "setup-code");
        assert!(Path::new(CODE_DIR).join(CODE_FILE).starts_with("/run"));
    }

    #[test]
    fn write_refuses_to_overwrite_a_live_code() {
        let dir = scratch_dir("excl");
        let path = dir.join("code");
        write_code_file(&path, CODE_A).unwrap();
        // O_EXCL: a second create must fail rather than replace a live code.
        assert!(write_code_file(&path, CODE_B).is_err());
        // The live code is unchanged.
        assert_eq!(fs::read_to_string(&path).unwrap(), CODE_A);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn consume_durable_removes_the_file_and_is_idempotent() {
        let dir = scratch_dir("consume");
        let path = dir.join("code");
        write_code_file(&path, CODE_A).unwrap();
        assert!(path.exists());
        consume_durable_at(&path).unwrap();
        assert!(!path.exists());
        // A second consume is a no-op (never an error).
        consume_durable_at(&path).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_code_verifies_false() {
        // Deterministic: a code path that does not exist must never verify
        // (the old conditional form could pass vacuously).
        let dir = scratch_dir("missing");
        let path = dir.join("no-such-code");
        assert!(!verify_at("anything", &path, current_uid()));
        let _ = fs::remove_dir_all(&dir);
    }

    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lqos-setup-code-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        // The production dir is 0700; the request path checks it on every read.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    fn current_uid() -> u32 {
        unsafe { libc::getuid() }
    }

    #[test]
    fn a_written_code_verifies_and_a_wrong_code_does_not() {
        let dir = scratch_dir("verify");
        let path = dir.join("code");
        write_code_file(&path, CODE_A).unwrap();
        assert!(verify_at(CODE_A, &path, current_uid()));
        assert!(!verify_at(CODE_B, &path, current_uid()));
        assert!(!verify_at("", &path, current_uid()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_short_stored_code_is_refused() {
        // A root-only file shortened to 8 chars must not verify an 8-char input:
        // the stored code has a FIXED length.
        let dir = scratch_dir("short");
        let path = dir.join("code");
        write_code_file(&path, "deadbeef").unwrap();
        assert!(!verify_at("deadbeef", &path, current_uid()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_mode_is_refused() {
        let dir = scratch_dir("mode");
        let path = dir.join("code");
        write_code_file(&path, CODE_A).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!verify_at(CODE_A, &path, current_uid()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_mode_directory_is_refused() {
        // The directory is part of the trust boundary, not just the file.
        let dir = scratch_dir("dirmode");
        let path = dir.join("code");
        write_code_file(&path, CODE_A).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!verify_at(CODE_A, &path, current_uid()));
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_owner_is_refused() {
        // The parent directory owner is checked BEFORE the file owner, so this
        // exercises the dir-owner gate (the dir is owned by current_uid, and
        // verification is asked for a different uid).
        let dir = scratch_dir("owner");
        let path = dir.join("code");
        write_code_file(&path, CODE_A).unwrap();
        let wrong_uid = current_uid().wrapping_add(1);
        assert!(!verify_at(CODE_A, &path, wrong_uid));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symlink_is_refused() {
        let dir = scratch_dir("symlink");
        let real = dir.join("real");
        write_code_file(&real, CODE_A).unwrap();
        let link = dir.join("code");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(!verify_at(CODE_A, &link, current_uid()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_file_is_refused() {
        let dir = scratch_dir("empty");
        let path = dir.join("code");
        write_code_file(&path, "   ").unwrap();
        assert!(!verify_at("", &path, current_uid()));
        assert!(!verify_at("anything", &path, current_uid()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_generation_is_distinguishable_and_marks_unavailable() {
        // The latch is process-global: hold the shared runtime-config lock so
        // this cannot race a handler test's reset window.
        let _guard = crate::test_support::runtime_config_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // A directory whose parent is a regular file cannot be created; that is
        // the startup failure the handler must refuse on (never a stale code).
        let dir = scratch_dir("genfail");
        let blocker = dir.join("not-a-dir");
        fs::write(&blocker, b"x").unwrap();
        let target = blocker.join("first_run");
        assert!(generate_in_dir(&target).is_err());
        mark_unavailable();
        assert!(is_unavailable());
        reset_latch_for_test();
        assert!(!is_unavailable());
        let _ = fs::remove_dir_all(&dir);
    }
}
