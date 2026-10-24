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


if __name__ == "__main__":
    unittest.main()
