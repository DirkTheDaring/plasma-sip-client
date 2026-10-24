#!/usr/bin/env python3
"""
Unit test suite verifying the exact production implementation of:
- .github/scripts/validate_tag.py
- .github/scripts/discover_fedora.py
"""

import sys
import unittest
import rpm

from validate_tag import validate_tag_string, compute_rpm_fields, validate_against_cargo
from discover_fedora import resolve_matrix
from publish_release import (
    parse_semver,
    is_higher_version,
    validate_and_stage_deliverables,
    generate_release_notes,
    compute_sha256,
)


class TestReleaseValidation(unittest.TestCase):

    def test_invalid_semver_tags_rejected(self):
        # Explicit test cases from requirements:
        invalid_tags = [
            "v1.0.0-alpha..1",   # consecutive dots in prerelease
            "v1.0.0-01",         # leading zero in numeric prerelease
            "v1.0.0-",           # empty prerelease after hyphen
            "v1.0.0+build..1",   # consecutive dots in build metadata
            "1.0.0",             # missing leading 'v'
            "v1.0",              # missing patch number
            "v1.0.0.0",          # 4-part version
            "",                  # empty string
        ]
        for tag in invalid_tags:
            is_valid, _, _, _, _, _, err = validate_tag_string(tag)
            self.assertFalse(is_valid, f"Expected tag '{tag}' to be rejected, but it was accepted")
            self.assertIsNotNone(err)

    def test_valid_semver_tags_accepted(self):
        valid_tags = [
            ("v1.0.0", "1", "0", "0", None, None),
            ("v1.1.0-alpha.1", "1", "1", "0", "alpha.1", None),
            ("v1.0.0-beta.2", "1", "0", "0", "beta.2", None),
            ("v1.0.0-rc.1", "1", "0", "0", "rc.1", None),
            ("v1.0.0-0.3.7", "1", "0", "0", "0.3.7", None),
            ("v1.0.0-x.7.z.92", "1", "0", "0", "x.7.z.92", None),
            ("v1.0.0+20130313144700", "1", "0", "0", None, "20130313144700"),
            ("v1.0.0-beta+exp.sha.5114f85", "1", "0", "0", "beta", "exp.sha.5114f85"),
        ]
        for tag, maj, min_, pat, pre, meta in valid_tags:
            is_valid, major, minor, patch, prerelease, build_meta, err = validate_tag_string(tag)
            self.assertTrue(is_valid, f"Expected tag '{tag}' to be accepted: {err}")
            self.assertEqual(major, maj)
            self.assertEqual(minor, min_)
            self.assertEqual(patch, pat)
            self.assertEqual(prerelease, pre)
            self.assertEqual(build_meta, meta)

    def test_rpm_prerelease_fields_and_sorting(self):
        # Generate RPM fields
        ver_alpha, rel_alpha = compute_rpm_fields("1", "0", "0", "alpha.1")
        ver_beta, rel_beta   = compute_rpm_fields("1", "0", "0", "beta.2")
        ver_rc, rel_rc       = compute_rpm_fields("1", "0", "0", "rc.1")
        ver_final, rel_final = compute_rpm_fields("1", "0", "0", None)

        self.assertEqual(ver_alpha, "1.0.0")
        self.assertEqual(rel_alpha, "0.1.alpha.1")
        self.assertEqual(ver_beta, "1.0.0")
        self.assertEqual(rel_beta, "0.2.beta.2")
        self.assertEqual(ver_rc, "1.0.0")
        self.assertEqual(rel_rc, "0.3.rc.1")
        self.assertEqual(ver_final, "1.0.0")
        self.assertEqual(rel_final, "1")

        # Verify exact RPM sorting order using native librpm labelCompare:
        # alpha < beta < rc < final
        fc = ".fc44"
        cmp_ab = rpm.labelCompare(("0", ver_alpha, rel_alpha + fc), ("0", ver_beta, rel_beta + fc))
        cmp_br = rpm.labelCompare(("0", ver_beta, rel_beta + fc), ("0", ver_rc, rel_rc + fc))
        cmp_rf = rpm.labelCompare(("0", ver_rc, rel_rc + fc), ("0", ver_final, rel_final + fc))
        cmp_af = rpm.labelCompare(("0", ver_alpha, rel_alpha + fc), ("0", ver_final, rel_final + fc))

        self.assertEqual(cmp_ab, -1, "alpha must sort before beta")
        self.assertEqual(cmp_br, -1, "beta must sort before rc")
        self.assertEqual(cmp_rf, -1, "rc must sort before final")
        self.assertEqual(cmp_af, -1, "alpha must sort before final")

    def test_cargo_manifest_validation(self):
        # Test against current Cargo.toml
        cargo_ver = validate_against_cargo("Cargo.toml", "1.0.0", "1.0.0")
        self.assertEqual(cargo_ver, "1.0.0")

        # Prerelease tag agreeing on base version
        cargo_ver_pre = validate_against_cargo("Cargo.toml", "1.0.0-alpha.1", "1.0.0")
        self.assertEqual(cargo_ver_pre, "1.0.0")

        # Mismatch must raise ValueError
        with self.assertRaises(ValueError):
            validate_against_cargo("Cargo.toml", "2.0.0", "2.0.0")

    def test_fedora_discovery_matrix_resolution(self):
        # Valid Bodhi payload with current Fedora releases
        mock_bodhi = {
            "releases": [
                {"id_prefix": "FEDORA", "name": "F44", "state": "current", "version": "44"},
                {"id_prefix": "FEDORA", "name": "F43", "state": "current", "version": "43"},
                {"id_prefix": "FEDORA-EPEL", "name": "EPEL-9", "state": "current", "version": "9"},
                {"id_prefix": "FEDORA", "name": "F44C", "state": "current", "version": "44"},
                {"id_prefix": "FEDORA", "name": "F45", "state": "pending", "version": "45"},
            ]
        }
        latest_n, matrix = resolve_matrix(mock_bodhi)
        self.assertEqual(latest_n, 44)
        self.assertEqual(matrix, ["44", "43", "42"])

        # Error cases
        with self.assertRaises(ValueError):
            resolve_matrix({"releases": []})

        with self.assertRaises(ValueError):
            resolve_matrix({"releases": [{"id_prefix": "FEDORA-EPEL", "name": "EPEL-10", "state": "current", "version": "10"}]})


class TestPublishRelease(unittest.TestCase):

    def test_semver_parsing_and_comparison(self):
        # Valid semver
        self.assertEqual(parse_semver("v1.0.0"), (1, 0, 0, True, ""))
        self.assertEqual(parse_semver("1.0.0"), (1, 0, 0, True, ""))
        self.assertEqual(parse_semver("v1.2.3-rc.1"), (1, 2, 3, False, "rc.1"))

        # Version comparisons
        self.assertTrue(is_higher_version("v1.1.0", "v1.0.0"))
        self.assertTrue(is_higher_version("v1.0.1", "v1.0.0"))
        self.assertTrue(is_higher_version("v1.0.0", "v1.0.0-rc.1"))
        self.assertTrue(is_higher_version("v1.0.0-rc.2", "v1.0.0-rc.1"))
        self.assertFalse(is_higher_version("v1.0.0", "v1.1.0"))
        self.assertFalse(is_higher_version("v1.0.0", "v1.0.0"))

    def test_release_notes_generation(self):
        notes = generate_release_notes(
            tag="v1.0.0",
            commit="f792c2e61d401e1fdaa480446d3cd88b547692eb",
            fedora_versions=["44", "43", "42"],
            workflow_run_url="https://github.com/DirkTheDaring/plasma-sip-client/actions/runs/40113091755",
            rpm_version="1.0.0",
            rpm_release="1",
        )
        self.assertIn("Plasma SIP Softphone v1.0.0", notes)
        self.assertIn("plasma-sip-1.0.0-1.fc44.x86_64.rpm", notes)
        self.assertIn("plasma-sip-1.0.0-1.fc43.x86_64.rpm", notes)
        self.assertIn("plasma-sip-1.0.0-1.fc42.x86_64.rpm", notes)
        self.assertIn("f792c2e61d401e1fdaa480446d3cd88b547692eb", notes)
        self.assertIn("Testing Boundaries & Known Limitations", notes)
        self.assertIn("sudo dnf install", notes)

    def test_deliverable_staging_and_verification(self):
        import tempfile
        import json
        from pathlib import Path

        with tempfile.TemporaryDirectory() as tmp_dir:
            base_path = Path(tmp_dir)
            artifacts_dir = base_path / "artifacts"
            staging_dir = base_path / "staging"
            artifacts_dir.mkdir()

            fedora_matrix = ["44", "43", "42"]
            tag = "v1.0.0"
            commit = "abc123commit"
            rpm_ver = "1.0.0"
            rpm_rel = "1"

            # Create mock artifact tree
            for ver in fedora_matrix:
                f_dir = artifacts_dir / f"fedora{ver}-rpms"
                f_dir.mkdir()
                rpms = [
                    f"plasma-sip-{rpm_ver}-{rpm_rel}.fc{ver}.x86_64.rpm",
                    f"plasma-sip-{rpm_ver}-{rpm_rel}.fc{ver}.src.rpm",
                    f"plasma-sip-debuginfo-{rpm_ver}-{rpm_rel}.fc{ver}.x86_64.rpm",
                    f"plasma-sip-debugsource-{rpm_ver}-{rpm_rel}.fc{ver}.x86_64.rpm",
                ]
                sums = []
                for r in rpms:
                    r_path = f_dir / r
                    content = f"dummy binary content for {r}".encode()
                    r_path.write_bytes(content)
                    digest = compute_sha256(r_path)
                    sums.append(f"{digest}  build/rpm/RPMS/x86_64/{r}\n")

                (f_dir / "SHA256SUMS").write_text("".join(sums))
                meta = {
                    "project": "plasma-sip",
                    "tag": tag,
                    "commit": commit,
                    "fedora_version": ver,
                    "architecture": "x86_64",
                }
                (f_dir / f"build-info-fedora{ver}.json").write_text(json.dumps(meta))

            # Run validation and staging
            staged = validate_and_stage_deliverables(
                artifacts_dir=artifacts_dir,
                staging_dir=staging_dir,
                fedora_versions=fedora_matrix,
                tag=tag,
                commit=commit,
                rpm_version=rpm_ver,
                rpm_release=rpm_rel,
            )

            # Staged deliverables: 3 fedoras * 4 rpms = 12 rpms + 3 metadata + 1 SHA256SUMS = 16 files
            self.assertEqual(len(staged), 16)
            staged_names = {p.name for p in staged}

            # Check individual RPMs present
            for ver in fedora_matrix:
                self.assertIn(f"plasma-sip-{rpm_ver}-{rpm_rel}.fc{ver}.x86_64.rpm", staged_names)
                self.assertIn(f"plasma-sip-{rpm_ver}-{rpm_rel}.fc{ver}.src.rpm", staged_names)
                self.assertIn(f"plasma-sip-debuginfo-{rpm_ver}-{rpm_rel}.fc{ver}.x86_64.rpm", staged_names)
                self.assertIn(f"plasma-sip-debugsource-{rpm_ver}-{rpm_rel}.fc{ver}.x86_64.rpm", staged_names)
                self.assertIn(f"build-info-fedora{ver}.json", staged_names)

            # Check SHA256SUMS file exists and contains filenames WITHOUT directory prefixes
            self.assertIn("SHA256SUMS", staged_names)
            sums_content = (staging_dir / "SHA256SUMS").read_text()
            for line in sums_content.strip().splitlines():
                parts = line.split()
                self.assertEqual(len(parts), 2)
                fname = parts[1]
                self.assertNotIn("/", fname, f"Checksum file path must not contain slashes: {fname}")
                self.assertTrue((staging_dir / fname).exists(), f"Asset {fname} must exist in staging")

    def test_deliverable_staging_validation_failures(self):
        import tempfile
        import json
        from pathlib import Path

        with tempfile.TemporaryDirectory() as tmp_dir:
            base_path = Path(tmp_dir)
            artifacts_dir = base_path / "artifacts"
            staging_dir = base_path / "staging"
            artifacts_dir.mkdir()

            f_dir = artifacts_dir / "fedora44-rpms"
            f_dir.mkdir()

            # Missing files error
            with self.assertRaises(FileNotFoundError):
                validate_and_stage_deliverables(
                    artifacts_dir=artifacts_dir,
                    staging_dir=staging_dir,
                    fedora_versions=["44"],
                    tag="v1.0.0",
                    commit="commit1",
                    rpm_version="1.0.0",
                    rpm_release="1",
                )

            # Create mock rpms
            for r in [
                "plasma-sip-1.0.0-1.fc44.x86_64.rpm",
                "plasma-sip-1.0.0-1.fc44.src.rpm",
                "plasma-sip-debuginfo-1.0.0-1.fc44.x86_64.rpm",
                "plasma-sip-debugsource-1.0.0-1.fc44.x86_64.rpm",
            ]:
                (f_dir / r).write_bytes(b"content")

            # Mismatched commit in metadata
            bad_meta = {
                "project": "plasma-sip",
                "tag": "v1.0.0",
                "commit": "different_commit",
                "fedora_version": "44",
                "architecture": "x86_64",
            }
            (f_dir / "build-info-fedora44.json").write_text(json.dumps(bad_meta))

            with self.assertRaises(ValueError) as ctx:
                validate_and_stage_deliverables(
                    artifacts_dir=artifacts_dir,
                    staging_dir=staging_dir,
                    fedora_versions=["44"],
                    tag="v1.0.0",
                    commit="commit1",
                    rpm_version="1.0.0",
                    rpm_release="1",
                )
            self.assertIn("commit mismatch", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()

