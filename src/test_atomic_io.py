import os
import stat
import tempfile
import unittest
from unittest.mock import patch

import atomic_io


class TestAtomicWrite(unittest.TestCase):
    def setUp(self):
        temp_dir = tempfile.TemporaryDirectory()  # nosec B108
        self.addCleanup(temp_dir.cleanup)
        self.temp_dir = temp_dir.name

    def test_write_text_creates_parents_and_content(self):
        path = os.path.join(self.temp_dir, "a", "b", "state.txt")
        atomic_io.atomic_write_text(path, "hello")
        with open(path, encoding="utf-8") as handle:
            self.assertEqual(handle.read(), "hello")
        self.assertFalse(os.path.exists(path + ".tmp"))

    def test_write_text_replaces_existing_content(self):
        path = os.path.join(self.temp_dir, "state.txt")
        with open(path, "w", encoding="utf-8") as handle:
            handle.write("old")
        atomic_io.atomic_write_text(path, "new")
        with open(path, encoding="utf-8") as handle:
            self.assertEqual(handle.read(), "new")
        self.assertFalse(os.path.exists(path + ".tmp"))

    def test_write_text_preserves_existing_permissions(self):
        path = os.path.join(self.temp_dir, "state.txt")
        with open(path, "w", encoding="utf-8") as handle:
            handle.write("old")
        os.chmod(path, 0o600)
        atomic_io.atomic_write_text(path, "new")
        self.assertEqual(stat.S_IMODE(os.stat(path).st_mode), 0o600)

    def test_write_text_failure_preserves_existing_content(self):
        path = os.path.join(self.temp_dir, "state.txt")
        with open(path, "w", encoding="utf-8") as handle:
            handle.write("old")
        with patch("os.replace", side_effect=OSError(5, "replace failed")):
            with self.assertRaises(OSError):
                atomic_io.atomic_write_text(path, "new")
        with open(path, encoding="utf-8") as handle:
            self.assertEqual(handle.read(), "old")
        self.assertFalse(os.path.exists(path + ".tmp"))


class TestAtomicCopy(unittest.TestCase):
    def setUp(self):
        temp_dir = tempfile.TemporaryDirectory()  # nosec B108
        self.addCleanup(temp_dir.cleanup)
        self.temp_dir = temp_dir.name

    def test_copy_creates_parents_and_preserves_bytes(self):
        src = os.path.join(self.temp_dir, "source.bin")
        with open(src, "wb") as handle:
            handle.write(b"\x00\xffpayload")
        dest = os.path.join(self.temp_dir, "nested", "copy.bin")
        atomic_io.atomic_copy(src, dest)
        with open(dest, "rb") as handle:
            self.assertEqual(handle.read(), b"\x00\xffpayload")
        self.assertFalse(os.path.exists(dest + ".tmp"))

    def test_copy_failure_preserves_existing_destination(self):
        src = os.path.join(self.temp_dir, "source.bin")
        with open(src, "wb") as handle:
            handle.write(b"new")
        dest = os.path.join(self.temp_dir, "dest.bin")
        with open(dest, "wb") as handle:
            handle.write(b"old")
        with patch("os.replace", side_effect=OSError(5, "replace failed")):
            with self.assertRaises(OSError):
                atomic_io.atomic_copy(src, dest)
        with open(dest, "rb") as handle:
            self.assertEqual(handle.read(), b"old")
        self.assertFalse(os.path.exists(dest + ".tmp"))

    def test_copy_missing_source_preserves_existing_destination(self):
        dest = os.path.join(self.temp_dir, "dest.bin")
        with open(dest, "wb") as handle:
            handle.write(b"old")
        missing = os.path.join(self.temp_dir, "does-not-exist.bin")
        with self.assertRaises(FileNotFoundError):
            atomic_io.atomic_copy(missing, dest)
        with open(dest, "rb") as handle:
            self.assertEqual(handle.read(), b"old")
        self.assertFalse(os.path.exists(dest + ".tmp"))
