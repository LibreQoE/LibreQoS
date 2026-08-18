import os
import pathlib
import subprocess
import tempfile
import textwrap
import unittest


SCRIPT = pathlib.Path(__file__).with_name("systemd_hotfix.sh")


class SystemdHotfixScriptTest(unittest.TestCase):
    def run_script_function(self, function_name, extra_env=None):
        with tempfile.TemporaryDirectory(prefix="lqos-hotfix-test.") as tmp:
            tmp_path = pathlib.Path(tmp)
            bin_path = tmp_path / "bin"
            key_path = tmp_path / "key" / "libreqos.gpg"
            source_path = tmp_path / "etc" / "apt" / "sources.list.d" / "libreqos-systemd-hotfix.list"
            preferences_path = tmp_path / "etc" / "apt" / "preferences.d" / "libreqos-systemd-hotfix"
            marker_path = tmp_path / "marker"
            apt_log_path = tmp_path / "apt.log"
            test_script_path = tmp_path / "systemd_hotfix_test.sh"

            bin_path.mkdir()
            key_path.parent.mkdir(parents=True)
            source_path.parent.mkdir(parents=True)
            preferences_path.parent.mkdir(parents=True)

            self.write_command(
                bin_path / "sudo",
                """
                #!/bin/sh
                exec "$@"
                """,
            )
            self.write_command(
                bin_path / "curl",
                """
                #!/bin/sh
                printf 'fake-keyring\\n'
                """,
            )
            self.write_command(
                bin_path / "systemctl",
                """
                #!/bin/sh
                case "$1" in
                  is-enabled) echo enabled ;;
                  is-active) echo active ;;
                  *) exit 1 ;;
                esac
                """,
            )
            self.write_command(
                bin_path / "dpkg-query",
                """
                #!/bin/sh
                case "$*" in
                  *'${Version}'*' systemd'*) echo '255.4-1ubuntu8.16' ;;
                  *'${db:Status-Abbrev}'*' libpam-systemd'*) echo ii ;;
                  *'${db:Status-Abbrev}'*' libnss-systemd'*) echo ii ;;
                  *'${db:Status-Abbrev}'*) echo rc ;;
                  *) exit 1 ;;
                esac
                """,
            )
            self.write_command(
                bin_path / "apt-get",
                """
                #!/bin/sh
                printf 'apt-get %s\\n' "$*" >> "$HOTFIX_TEST_APT_LOG"
                """,
            )
            self.write_command(
                bin_path / "apt-cache",
                """
                #!/bin/sh
                [ "${HOTFIX_TEST_APT_CACHE_FAIL:-0}" = "1" ] && exit 12

                package="$2"
                version="${HOTFIX_TEST_CANDIDATE_VERSION:-255.4-1ubuntu9999+libreqos1}"
                priority="${HOTFIX_TEST_CANDIDATE_PRIORITY:-1001}"
                repo_line="${HOTFIX_TEST_REPO_LINE:-        500 https://repo.libreqos.com noble/main amd64 Packages}"
                release_line="${HOTFIX_TEST_RELEASE_LINE:-}"

                case ",${HOTFIX_TEST_INCONSISTENT_PACKAGES:-}," in
                  *,"$package",*) version="${HOTFIX_TEST_OTHER_VERSION:-255.4-1ubuntu9999+libreqos2}" ;;
                esac

                if [ "${HOTFIX_TEST_NO_CANDIDATE_FOR:-}" = "$package" ]; then
                  version="(none)"
                fi

                cat <<POLICY
                $package:
                  Installed: 255.4-1ubuntu8.16
                  Candidate: $version
                  Version table:
                     $version $priority
                $repo_line
                $release_line
                     255.4-1ubuntu8.16 500
                        500 http://archive.ubuntu.com/ubuntu noble-updates/main amd64 Packages
                POLICY
                """,
            )

            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{bin_path}{os.pathsep}{env['PATH']}",
                    "HOTFIX_TEST_APT_LOG": str(apt_log_path),
                    "HOTFIX_SKIP_REBOOT_PROMPT": "1",
                    "HOTFIX_KEYRING_PATH": str(key_path),
                    "HOTFIX_APT_SOURCE_PATH": str(source_path),
                    "HOTFIX_APT_PREFERENCES_PATH": str(preferences_path),
                    "HOTFIX_MARKER": str(marker_path),
                }
            )
            if extra_env:
                env.update(extra_env)

            test_script_path.write_text(SCRIPT.read_text().replace('main "$@"', ":"))
            command = (
                "source \"$HOTFIX_TEST_SCRIPT\"; "
                "is_supported_os() { return 0; }; "
                f"{function_name}"
            )
            env["HOTFIX_TEST_SCRIPT"] = str(test_script_path)
            result = subprocess.run(
                ["bash", "-c", command],
                cwd=SCRIPT.parent,
                env=env,
                text=True,
                capture_output=True,
                check=False,
            )

            apt_log = apt_log_path.read_text() if apt_log_path.exists() else ""
            marker = marker_path.read_text() if marker_path.exists() else ""
            return result, apt_log, marker

    def run_install(self, extra_env=None):
        return self.run_script_function("install_bundle", extra_env)

    @staticmethod
    def write_command(path, content):
        path.write_text(textwrap.dedent(content).lstrip())
        path.chmod(0o755)

    def test_auto_resolves_consistent_libreqos_candidate(self):
        result, apt_log, marker = self.run_install()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(
            "Resolved LibreQoS systemd hotfix package version: 255.4-1ubuntu9999+libreqos1",
            result.stdout,
        )
        self.assertIn("apt-get update", apt_log)
        self.assertIn("systemd=255.4-1ubuntu9999+libreqos1", apt_log)
        self.assertIn("libpam-systemd=255.4-1ubuntu9999+libreqos1", apt_log)
        self.assertIn("libnss-systemd=255.4-1ubuntu9999+libreqos1", apt_log)
        self.assertNotIn("libnss-resolve=", apt_log)
        self.assertIn("package_version=255.4-1ubuntu9999+libreqos1", marker)

    def test_auto_accepts_policy_repo_url_with_trailing_slash(self):
        result, apt_log, marker = self.run_install(
            {
                "HOTFIX_TEST_REPO_LINE": (
                    "        500 https://repo.libreqos.com/ noble/main amd64 Packages"
                ),
            }
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("systemd=255.4-1ubuntu9999+libreqos1", apt_log)
        self.assertIn("package_version=255.4-1ubuntu9999+libreqos1", marker)

    def test_auto_uses_policy_release_metadata_when_source_component_is_unrecognized(self):
        result, apt_log, marker = self.run_install(
            {
                "HOTFIX_TEST_REPO_LINE": (
                    "        500 https://repo.libreqos.com noble amd64 Packages"
                ),
                "HOTFIX_TEST_RELEASE_LINE": (
                    "        release o=LibreQoS,n=noble,l=LibreQoS,c=main,b=amd64"
                ),
            }
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("systemd=255.4-1ubuntu9999+libreqos1", apt_log)
        self.assertIn("package_version=255.4-1ubuntu9999+libreqos1", marker)

    def test_auto_rejects_ubuntu_candidate(self):
        result, _, _ = self.run_install(
            {"HOTFIX_TEST_CANDIDATE_VERSION": "255.4-1ubuntu8.16"}
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("LibreQoS hotfix candidate is not available", result.stderr)

    def test_auto_rejects_missing_candidate(self):
        result, _, _ = self.run_install(
            {"HOTFIX_TEST_NO_CANDIDATE_FOR": "systemd"}
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("No APT candidate is available for systemd", result.stderr)

    def test_auto_rejects_inconsistent_package_versions(self):
        result, _, _ = self.run_install(
            {"HOTFIX_TEST_INCONSISTENT_PACKAGES": "udev"}
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Inconsistent LibreQoS hotfix package versions", result.stderr)

    def test_auto_rejects_low_priority_candidate(self):
        result, _, _ = self.run_install(
            {"HOTFIX_TEST_CANDIDATE_PRIORITY": "500"}
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not pinned from the LibreQoS hotfix repo", result.stderr)

    def test_auto_rejects_candidate_from_unexpected_repo(self):
        result, _, _ = self.run_install(
            {
                "HOTFIX_TEST_REPO_LINE": (
                    "        500 https://example.invalid noble/main amd64 Packages"
                ),
            }
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("is not from https://repo.libreqos.com noble/main", result.stderr)

    def test_auto_rejects_candidate_from_prefix_confusable_repo(self):
        result, _, _ = self.run_install(
            {
                "HOTFIX_TEST_REPO_LINE": (
                    "        500 https://repo.libreqos.com.evil noble/main amd64 Packages"
                ),
            }
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("is not from https://repo.libreqos.com noble/main", result.stderr)

    def test_auto_rejects_release_metadata_from_different_source_stanza(self):
        result, _, _ = self.run_install(
            {
                "HOTFIX_TEST_REPO_LINE": textwrap.dedent(
                    """
                            500 https://repo.libreqos.com noble amd64 Packages
                            release o=Mirror,n=noble,l=Mirror,c=main,b=amd64
                            500 https://mirror.invalid noble/main amd64 Packages
                            release o=LibreQoS,n=noble,l=LibreQoS,c=main,b=amd64
                    """
                ).rstrip(),
            }
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("is not from https://repo.libreqos.com noble/main", result.stderr)

    def test_auto_accepts_release_metadata_after_unrelated_source_stanza(self):
        result, apt_log, marker = self.run_install(
            {
                "HOTFIX_TEST_REPO_LINE": textwrap.dedent(
                    """
                            500 https://mirror.invalid noble/main amd64 Packages
                            release o=Mirror,n=noble,l=Mirror,c=main,b=amd64
                            500 https://repo.libreqos.com noble amd64 Packages
                            release o=LibreQoS,n=noble,l=LibreQoS,c=main,b=amd64
                    """
                ).rstrip(),
            }
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("systemd=255.4-1ubuntu9999+libreqos1", apt_log)
        self.assertIn("package_version=255.4-1ubuntu9999+libreqos1", marker)

    def test_exact_override_bypasses_candidate_resolution(self):
        result, apt_log, marker = self.run_install(
            {
                "HOTFIX_PACKAGE_VERSION": "255.4-1ubuntu9999+libreqos1",
                "HOTFIX_TEST_APT_CACHE_FAIL": "1",
            }
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("systemd=255.4-1ubuntu9999+libreqos1", apt_log)
        self.assertIn("package_version=255.4-1ubuntu9999+libreqos1", marker)

    def test_download_uses_resolved_exact_package_specs(self):
        result, apt_log, marker = self.run_script_function("download_bundle")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("apt-get update", apt_log)
        self.assertIn("apt-get download systemd=255.4-1ubuntu9999+libreqos1", apt_log)
        self.assertEqual("", marker)


if __name__ == "__main__":
    unittest.main()
