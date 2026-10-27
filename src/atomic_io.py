"""Atomic file writes for runtime state and generated shaping inputs.

Each write goes to a sibling temporary file that is flushed and fsync'd before
an os.replace() swaps it over the destination. A reader therefore never sees a
truncated or torn file when a write is interrupted by a crash, VM snapshot, or
power loss; the worst case is the previous complete file.

A single writer is assumed per destination path; concurrent writers to the same
path are not coordinated.
"""

import os
import shutil
import stat


def _ensure_parent_dir(path):
    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)


def _remove_temp_file(temp_path):
    try:
        os.remove(temp_path)
    except OSError:
        pass


def _preserve_destination_metadata(path, temp_path):
    # Replacing a file creates a new inode, so carry over the destination's
    # ownership and permissions. Operators edit generated files over SFTP, and
    # the scheduler runs as root.
    try:
        existing = os.stat(path)
    except OSError:
        return
    try:
        os.chmod(temp_path, stat.S_IMODE(existing.st_mode))
    except OSError:
        pass
    try:
        os.chown(temp_path, existing.st_uid, existing.st_gid)
    except OSError:
        pass


def _atomic_replace(path, write_temp):
    _ensure_parent_dir(path)
    temp_path = path + ".tmp"
    replaced = False
    try:
        with open(temp_path, "wb") as handle:
            write_temp(handle)
            handle.flush()
            os.fsync(handle.fileno())
        _preserve_destination_metadata(path, temp_path)
        os.replace(temp_path, path)
        replaced = True
    finally:
        if not replaced:
            _remove_temp_file(temp_path)


def atomic_write_text(path, text, encoding="utf-8"):
    """Atomically replace `path` with `text`.

    Side effects: creates parent directories, writes and fsyncs `path + ".tmp"`,
    preserves the destination's ownership and permissions when it exists, then
    renames the temporary file over `path`.
    """
    _atomic_replace(path, lambda handle: handle.write(text.encode(encoding)))


def atomic_copy(src, dest):
    """Atomically copy `src` to `dest`, preserving the exact bytes.

    Side effects match `atomic_write_text`.
    """

    def write_temp(handle):
        with open(src, "rb") as source:
            shutil.copyfileobj(source, handle)

    _atomic_replace(dest, write_temp)
