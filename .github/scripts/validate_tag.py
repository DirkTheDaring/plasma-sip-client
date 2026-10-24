#!/usr/bin/env python3
"""
Validates that the pushed Git tag is a valid Semantic Version 2.0.0
and matches the version specified in Cargo.toml.
Computes compliant RPM Version and Release fields.
"""

import os
import re
import sys
import tomllib

SEMVER_REGEX = re.compile(
    r"^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?"
    r"(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$"
)


def validate_tag_string(tag: str):
    """
    Validates tag string against SemVer 2.0.0 specification with leading 'v'.
    Returns (is_valid, major, minor, patch, prerelease, build_metadata, error_message).
    """
    if not tag:
        return False, None, None, None, None, None, "Tag cannot be empty"

    match = SEMVER_REGEX.match(tag)
    if not match:
        return (
            False,
            None,
            None,
            None,
            None,
            None,
            f"Tag '{tag}' is not a valid Semantic Version 2.0.0 string (e.g. v1.0.0 or v1.1.0-alpha.1)",
        )

    major = match.group(1)
    minor = match.group(2)
    patch = match.group(3)
    prerelease = match.group(4)
    build_meta = match.group(5)

    return True, major, minor, patch, prerelease, build_meta, None


def compute_rpm_fields(major: str, minor: str, patch: str, prerelease: str | None):
    """
    Computes RPM Version and Release fields adhering to Fedora Packaging Guidelines.
    Prerelease packages use '0.<order>.<prerelease>' in the Release field,
    guaranteeing they sort before the final release '1'.
    """
    rpm_version = f"{major}.{minor}.{patch}"
    if not prerelease:
        rpm_release = "1"
    else:
        # Determine Fedora prerelease sorting bucket
        lower_pre = prerelease.lower()
        if "alpha" in lower_pre:
            order = 1
        elif "beta" in lower_pre:
            order = 2
        elif "rc" in lower_pre:
            order = 3
        else:
            order = 1

        # Replace invalid RPM Release characters (only alphanumeric and dots allowed)
        clean_prerelease = re.sub(r"[^0-9a-zA-Z.]+", ".", prerelease).strip(".")
        rpm_release = f"0.{order}.{clean_prerelease}"

    return rpm_version, rpm_release


def validate_against_cargo(cargo_path: str, tag_version: str, base_version: str):
    """
    Verifies that the tag version agrees with Cargo.toml [workspace.package].version
    or [package].version.
    """
    if not os.path.isfile(cargo_path):
        raise FileNotFoundError(f"Cargo.toml not found at {cargo_path}")

    with open(cargo_path, "rb") as f:
        manifest = tomllib.load(f)

    cargo_version = manifest.get("workspace", {}).get("package", {}).get("version")
    if not cargo_version:
        cargo_version = manifest.get("package", {}).get("version")

    if not cargo_version:
        raise ValueError("Could not find version field in Cargo.toml")

    # Accept exact match (e.g. 1.0.0 or 1.0.0-alpha.1) or base version match for prereleases
    if tag_version != cargo_version and base_version != cargo_version:
        raise ValueError(
            f"Version mismatch: git tag version '{tag_version}' does not agree with Cargo.toml version '{cargo_version}'"
        )

    return cargo_version


def main():
    tag = os.environ.get("REF_NAME", "")
    if not tag:
        sys.exit("Error: REF_NAME environment variable not set")

    is_valid, major, minor, patch, prerelease, _, err = validate_tag_string(tag)
    if not is_valid:
        sys.exit(f"Error: {err}")

    tag_version = tag[1:]  # strip leading 'v'
    base_version = f"{major}.{minor}.{patch}"

    try:
        cargo_version = validate_against_cargo("Cargo.toml", tag_version, base_version)
    except Exception as e:
        sys.exit(f"Cargo manifest validation error: {e}")

    rpm_version, rpm_release = compute_rpm_fields(major, minor, patch, prerelease)

    print(f"Tag validated successfully: {tag}")
    print(f"  Cargo.toml version: {cargo_version}")
    print(f"  Base version:       {base_version}")
    print(f"  Prerelease:         {prerelease or 'none'}")
    print(f"  RPM Version:        {rpm_version}")
    print(f"  RPM Release:        {rpm_release}")

    github_output = os.environ.get("GITHUB_OUTPUT")
    if github_output:
        is_pre_str = "true" if prerelease else "false"
        with open(github_output, "a") as f:
            f.write(f"tag_version={tag_version}\n")
            f.write(f"base_version={base_version}\n")
            f.write(f"rpm_version={rpm_version}\n")
            f.write(f"rpm_release={rpm_release}\n")
            f.write(f"is_prerelease={is_pre_str}\n")


if __name__ == "__main__":
    main()
