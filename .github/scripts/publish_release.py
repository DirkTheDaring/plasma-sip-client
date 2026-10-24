#!/usr/bin/env python3
"""
Validates release deliverables across all dynamically resolved Fedora targets,
prepares unified release assets with combined SHA256 checksums, and manages
safe, idempotent GitHub Release creation and publication.
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

SEMVER_REGEX = re.compile(
    r"^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?"
    r"(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$"
)


def compute_sha256(filepath: Path) -> str:
    """Computes the SHA-256 hexadecimal digest of a file."""
    h = hashlib.sha256()
    with open(filepath, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


def parse_semver(v_str: str):
    """
    Parses a SemVer string into a tuple suitable for comparison.
    Returns (major, minor, patch, is_prerelease, prerelease_str).
    """
    match = SEMVER_REGEX.match(v_str.strip())
    if not match:
        raise ValueError(f"Invalid SemVer string: {v_str}")
    major = int(match.group(1))
    minor = int(match.group(2))
    patch = int(match.group(3))
    prerelease = match.group(4)
    is_prerelease = prerelease is not None and len(prerelease) > 0
    return (major, minor, patch, not is_prerelease, prerelease or "")


def is_higher_version(v_a: str, v_b: str) -> bool:
    """Returns True if SemVer v_a is strictly greater than v_b."""
    return parse_semver(v_a) > parse_semver(v_b)


def validate_and_stage_deliverables(
    artifacts_dir: Path,
    staging_dir: Path,
    fedora_versions: list[str],
    tag: str,
    commit: str,
    rpm_version: str,
    rpm_release: str,
) -> list[Path]:
    """
    Verifies that all target Fedora artifacts exist, have valid filenames,
    match build metadata (tag, commit, arch), pass internal checksum verification,
    and stages them cleanly for release upload.
    """
    staging_dir.mkdir(parents=True, exist_ok=True)
    staged_files: list[Path] = []

    print(f"==> Validating deliverables for Fedora targets: {fedora_versions}...")

    for ver in fedora_versions:
        ver_str = str(ver).strip()
        # Find directory matching this Fedora version
        target_dir = None
        for candidate in artifacts_dir.iterdir():
            if candidate.is_dir() and f"fedora{ver_str}" in candidate.name:
                target_dir = candidate
                break

        if not target_dir:
            raise FileNotFoundError(
                f"Missing artifact directory for Fedora {ver_str} in {artifacts_dir}"
            )

        print(f"--> Validating Fedora {ver_str} in {target_dir.name}...")

        # Locate RPM files
        all_rpms = list(target_dir.glob("**/*.rpm"))
        if not all_rpms:
            raise FileNotFoundError(f"No RPM packages found in {target_dir}")

        expected_main = f"plasma-sip-{rpm_version}-{rpm_release}.fc{ver_str}.x86_64.rpm"
        expected_src = f"plasma-sip-{rpm_version}-{rpm_release}.fc{ver_str}.src.rpm"
        expected_debuginfo = f"plasma-sip-debuginfo-{rpm_version}-{rpm_release}.fc{ver_str}.x86_64.rpm"
        expected_debugsource = f"plasma-sip-debugsource-{rpm_version}-{rpm_release}.fc{ver_str}.x86_64.rpm"

        rpm_by_name = {rpm.name: rpm for rpm in all_rpms}

        for expected in [expected_main, expected_src, expected_debuginfo, expected_debugsource]:
            if expected not in rpm_by_name:
                raise FileNotFoundError(
                    f"Fedora {ver_str}: Expected deliverable '{expected}' was not found in {target_dir}"
                )

        # Check for unexpected extra RPMs
        expected_set = {expected_main, expected_src, expected_debuginfo, expected_debugsource}
        extra_rpms = set(rpm_by_name.keys()) - expected_set
        if extra_rpms:
            raise ValueError(f"Fedora {ver_str}: Unexpected RPM deliverables found: {extra_rpms}")

        # Locate and validate build metadata
        meta_candidates = list(target_dir.glob("**/build-info*.json"))
        if not meta_candidates:
            raise FileNotFoundError(f"Fedora {ver_str}: Missing build-info JSON metadata in {target_dir}")

        meta_file = meta_candidates[0]
        with open(meta_file, "r", encoding="utf-8") as f:
            metadata = json.load(f)

        meta_tag = metadata.get("tag")
        meta_commit = metadata.get("commit")
        meta_fedora = str(metadata.get("fedora_version"))
        meta_arch = metadata.get("architecture")

        if meta_tag != tag:
            raise ValueError(
                f"Fedora {ver_str} metadata tag mismatch: expected '{tag}', found '{meta_tag}'"
            )
        if meta_commit != commit:
            raise ValueError(
                f"Fedora {ver_str} metadata commit mismatch: expected '{commit}', found '{meta_commit}'"
            )
        if meta_fedora != ver_str:
            raise ValueError(
                f"Fedora {ver_str} metadata version mismatch: expected '{ver_str}', found '{meta_fedora}'"
            )
        if meta_arch != "x86_64":
            raise ValueError(
                f"Fedora {ver_str} metadata arch mismatch: expected 'x86_64', found '{meta_arch}'"
            )

        # Validate against SHA256SUMS if present in the artifact
        checksum_files = list(target_dir.glob("**/SHA256SUMS"))
        if checksum_files:
            expected_hashes = {}
            with open(checksum_files[0], "r", encoding="utf-8") as f:
                for line in f:
                    parts = line.strip().split()
                    if len(parts) == 2:
                        basename = Path(parts[1]).name
                        expected_hashes[basename] = parts[0]

            for rpm_name, rpm_path in rpm_by_name.items():
                if rpm_name in expected_hashes:
                    actual_hash = compute_sha256(rpm_path)
                    if actual_hash != expected_hashes[rpm_name]:
                        raise ValueError(
                            f"Checksum mismatch for {rpm_name}: expected {expected_hashes[rpm_name]}, computed {actual_hash}"
                        )
                    print(f"    Verified SHA256 for {rpm_name}: {actual_hash[:12]}... OK")

        # Stage the 4 RPMs
        for rpm_name, rpm_path in rpm_by_name.items():
            dest = staging_dir / rpm_name
            shutil.copy2(rpm_path, dest)
            staged_files.append(dest)

        # Stage uniquely named build-info JSON
        unique_meta_name = f"build-info-fedora{ver_str}.json"
        dest_meta = staging_dir / unique_meta_name
        shutil.copy2(meta_file, dest_meta)
        staged_files.append(dest_meta)

    # Generate unified SHA256SUMS over all staged assets without directory prefixes
    unified_checksums_path = staging_dir / "SHA256SUMS"
    all_staged = sorted(staged_files, key=lambda p: p.name)
    with open(unified_checksums_path, "w", encoding="utf-8") as f:
        for p in all_staged:
            digest = compute_sha256(p)
            f.write(f"{digest}  {p.name}\n")

    staged_files.append(unified_checksums_path)
    print(f"==> Successfully staged {len(staged_files)} release deliverables in {staging_dir}:")
    for f in sorted(staged_files, key=lambda p: p.name):
        print(f"    {f.name} ({f.stat().st_size:,} bytes)")

    return staged_files


def generate_release_notes(
    tag: str,
    commit: str,
    fedora_versions: list[str],
    workflow_run_url: str,
    rpm_version: str,
    rpm_release: str,
) -> str:
    """Generates user-facing markdown release notes."""
    n_ver = fedora_versions[0]
    n_minus_1 = fedora_versions[1]
    n_minus_2 = fedora_versions[2]

    return f"""# Plasma SIP Softphone {tag}

Plasma SIP is a modern, pure Rust SIP softphone tailored specifically for KDE Plasma 6 on Linux, featuring native PipeWire audio integration, AVM FRITZ!Box PBX support, and generic SIP provider compatibility.

## Quick Installation Instructions (Fedora)

Download the RPM corresponding to your Fedora distribution, then install it using DNF:

### Fedora {n_ver} (Latest Stable)
```bash
sudo dnf install ./plasma-sip-{rpm_version}-{rpm_release}.fc{n_ver}.x86_64.rpm
```

### Fedora {n_minus_1}
```bash
sudo dnf install ./plasma-sip-{rpm_version}-{rpm_release}.fc{n_minus_1}.x86_64.rpm
```

### Fedora {n_minus_2}
```bash
sudo dnf install ./plasma-sip-{rpm_version}-{rpm_release}.fc{n_minus_2}.x86_64.rpm
```

---

## Release Verification & Build Details

- **Release Tag:** `{tag}`
- **Source Commit:** [`{commit}`]({workflow_run_url})
- **Authoritative Fedora Matrix:** Rolling three-version support window `{json.dumps(fedora_versions)}` resolved via the authoritative Fedora Bodhi API.
- **Workflow Run:** [Release Build Run]({workflow_run_url})
- **Deliverables & Checksums:** Checksums for all native packages and source RPMs are cataloged in `SHA256SUMS`. Build environment specifications for each target are attached as `build-info-fedora*.json`.

---

## Testing Boundaries & Known Limitations

- **Container Verification:** Application packages have been built, verified for file integrity (`rpm -V`), and smoke-tested in clean, disposable Fedora containers for all target versions.
- **Audio Hardware:** Container verification validates binary execution and library linkability; physical ALSA soundcard capture and playback via real PipeWire hardware graphs require host desktop session testing.
- **Telephony & PBX Services:** Live SIP network registration and RTP media streams were not tested against a live external PBX service in container CI.
- **Desktop Shell Integration:** Graphical Kirigami components were validated offscreen (`QT_QPA_PLATFORM=offscreen`). Live KDE Plasma Wayland system tray icon (`StatusNotifierItem`) and XDG activation token handshakes require an interactive desktop session.
"""


def run_gh_cmd(args: list[str]) -> subprocess.CompletedProcess:
    """Runs a gh CLI command and returns the CompletedProcess object."""
    cmd = ["gh"] + args
    return subprocess.run(cmd, capture_output=True, text=True, check=False)


def get_existing_release(tag: str) -> dict | None:
    """Queries gh release view for the specified tag."""
    res = run_gh_cmd(["release", "view", tag, "--json", "tagName,isDraft,isPrerelease,assets,url"])
    if res.returncode != 0:
        return None
    try:
        return json.loads(res.stdout)
    except Exception:
        return None


def determine_if_latest(tag: str, is_prerelease: bool) -> bool:
    """
    Determines whether this release should be marked 'Latest'.
    Never sets latest if it is a prerelease.
    For final releases, checks whether a higher SemVer release already exists.
    """
    if is_prerelease:
        return False

    res = run_gh_cmd(["release", "list", "--json", "tagName,isPrerelease,isDraft"])
    if res.returncode != 0:
        return True

    try:
        releases = json.loads(res.stdout)
    except Exception:
        return True

    current_semver = parse_semver(tag)
    for r in releases:
        if r.get("isDraft"):
            continue
        if r.get("isPrerelease"):
            continue
        existing_tag = r.get("tagName", "")
        try:
            if parse_semver(existing_tag) > current_semver:
                print(f"Notice: Existing release '{existing_tag}' is newer than '{tag}'. Not marking as Latest.")
                return False
        except ValueError:
            continue

    return True


def publish_github_release(
    tag: str,
    commit: str,
    staged_files: list[Path],
    notes_content: str,
    is_prerelease: bool,
    dry_run: bool = False,
) -> str:
    """
    Creates/resumes draft release, uploads verified assets, checks rerun safety,
    and publishes the final release.
    """
    staging_by_name = {f.name: f for f in staged_files}
    existing_release = get_existing_release(tag)

    if existing_release and not existing_release.get("isDraft", False):
        print(f"==> Published release for {tag} already exists.")
        # Rerun safety: verify published release assets match local staged assets
        remote_assets = {a["name"]: a for a in existing_release.get("assets", [])}
        missing_assets = set(staging_by_name.keys()) - set(remote_assets.keys())
        if missing_assets:
            raise RuntimeError(
                f"Published release for {tag} exists but is missing expected assets: {missing_assets}. "
                "Refusing to silently modify published release."
            )

        # Verify sizes
        for name, local_path in staging_by_name.items():
            remote_size = remote_assets[name]["size"]
            local_size = local_path.stat().st_size
            if remote_size != local_size:
                raise RuntimeError(
                    f"Published release asset '{name}' size mismatch: remote={remote_size}, local={local_size}. "
                    "Never silently overwrite or delete published packages!"
                )

        print(f"==> Existing release {tag} verified identical to current build. Safe rerun completed successfully.")
        return existing_release.get("url", f"https://github.com/DirkTheDaring/plasma-sip-client/releases/tag/{tag}")

    is_latest = determine_if_latest(tag, is_prerelease)

    notes_tmp = Path(f"/tmp/release_notes_{tag}.md")
    notes_tmp.write_text(notes_content, encoding="utf-8")

    try:
        if dry_run:
            print("==> [DRY RUN] Would execute GitHub Release creation and publication.")
            return f"https://github.com/DirkTheDaring/plasma-sip-client/releases/tag/{tag} (dry-run)"

        # Check if draft release already exists
        if existing_release and existing_release.get("isDraft", True):
            print(f"==> Found existing draft release for {tag}. Resuming upload...")
            remote_assets = {a["name"]: a for a in existing_release.get("assets", [])}
            for name, local_path in staging_by_name.items():
                if name in remote_assets:
                    if remote_assets[name]["size"] == local_path.stat().st_size:
                        print(f"    Asset '{name}' already present with matching size. Skipping upload.")
                        continue
                    else:
                        print(f"    Asset '{name}' exists with different size. Replacing on draft...")
                        run_gh_cmd(["release", "delete-asset", tag, name, "-y"])

                print(f"    Uploading '{name}' to draft release...")
                upload_res = run_gh_cmd(["release", "upload", tag, str(local_path)])
                if upload_res.returncode != 0:
                    raise RuntimeError(f"Failed to upload {name}: {upload_res.stderr}")
        else:
            print(f"==> Creating new draft release for {tag}...")
            create_args = [
                "release",
                "create",
                tag,
                "--draft",
                "--verify-tag",
                "--title",
                f"Plasma SIP {tag}",
                "--notes-file",
                str(notes_tmp),
            ]
            if is_prerelease:
                create_args.append("--prerelease")
            if is_latest:
                create_args.append("--latest=true")
            else:
                create_args.append("--latest=false")

            # Attach all staged files directly
            create_args.extend([str(f) for f in staged_files])

            res = run_gh_cmd(create_args)
            if res.returncode != 0:
                raise RuntimeError(f"Failed to create draft release: {res.stderr}\n{res.stdout}")

        # Verify all assets are attached to draft release
        print("==> Verifying complete asset set on draft release...")
        check_release = get_existing_release(tag)
        if not check_release:
            raise RuntimeError(f"Could not retrieve draft release for {tag} after creation")

        attached_assets = {a["name"]: a for a in check_release.get("assets", [])}
        missing_on_draft = set(staging_by_name.keys()) - set(attached_assets.keys())
        if missing_on_draft:
            raise RuntimeError(f"Draft release is missing expected assets: {missing_on_draft}")

        for name, local_path in staging_by_name.items():
            if attached_assets[name]["size"] != local_path.stat().st_size:
                raise RuntimeError(
                    f"Asset size mismatch on draft release for '{name}': remote={attached_assets[name]['size']}, local={local_path.stat().st_size}"
                )

        # Publish the release
        print(f"==> Publishing release {tag}...")
        publish_args = ["release", "edit", tag, "--draft=false"]
        if is_latest:
            publish_args.append("--latest=true")
        else:
            publish_args.append("--latest=false")

        pub_res = run_gh_cmd(publish_args)
        if pub_res.returncode != 0:
            raise RuntimeError(f"Failed to publish release: {pub_res.stderr}")

        published_release = get_existing_release(tag)
        release_url = published_release.get("url", f"https://github.com/DirkTheDaring/plasma-sip-client/releases/tag/{tag}")
        print(f"==> Successfully published release: {release_url}")
        return release_url

    finally:
        if notes_tmp.exists():
            notes_tmp.unlink()


def main():
    parser = argparse.ArgumentParser(description="Validate and publish GitHub Releases.")
    parser.add_argument("--tag", required=True, help="Git tag name (e.g. v1.0.0)")
    parser.add_argument("--commit", required=True, help="Expected source commit SHA")
    parser.add_argument("--artifacts-dir", required=True, help="Directory containing downloaded build artifacts")
    parser.add_argument("--workflow-run-url", required=True, help="GitHub Actions run URL")
    parser.add_argument("--fedora-matrix", required=True, help="JSON array of Fedora versions, e.g. '[\"44\", \"43\", \"42\"]'")
    parser.add_argument("--rpm-version", required=True, help="RPM Version (e.g. 1.0.0)")
    parser.add_argument("--rpm-release", required=True, help="RPM Release without dist (e.g. 1 or 0.1.alpha.3)")
    parser.add_argument("--is-prerelease", default="false", help="'true' if prerelease, 'false' otherwise")
    parser.add_argument("--staging-dir", default="release_staging", help="Directory to stage verified assets")
    parser.add_argument("--run-id", help="Optional GitHub Run ID to download artifacts from if artifacts-dir is empty")
    parser.add_argument("--dry-run", action="store_true", help="Perform validation and staging without publishing")

    args = parser.parse_args()

    artifacts_path = Path(args.artifacts_dir)
    staging_path = Path(args.staging_dir)
    fedora_versions = json.loads(args.fedora_matrix)
    is_prerelease = str(args.is_prerelease).lower() in ("true", "1", "yes")

    # If artifacts directory is empty or missing and run-id is provided, download via gh
    if (not artifacts_path.exists() or not any(artifacts_path.iterdir())) and args.run_id:
        print(f"==> Downloading artifacts from GitHub run {args.run_id} to {artifacts_path}...")
        artifacts_path.mkdir(parents=True, exist_ok=True)
        dl_res = run_gh_cmd(["run", "download", str(args.run_id), "--dir", str(artifacts_path)])
        if dl_res.returncode != 0:
            sys.exit(f"Failed to download artifacts from run {args.run_id}: {dl_res.stderr}")

    staged_files = validate_and_stage_deliverables(
        artifacts_dir=artifacts_path,
        staging_dir=staging_path,
        fedora_versions=fedora_versions,
        tag=args.tag,
        commit=args.commit,
        rpm_version=args.rpm_version,
        rpm_release=args.rpm_release,
    )

    notes = generate_release_notes(
        tag=args.tag,
        commit=args.commit,
        fedora_versions=fedora_versions,
        workflow_run_url=args.workflow_run_url,
        rpm_version=args.rpm_version,
        rpm_release=args.rpm_release,
    )

    release_url = publish_github_release(
        tag=args.tag,
        commit=args.commit,
        staged_files=staged_files,
        notes_content=notes,
        is_prerelease=is_prerelease,
        dry_run=args.dry_run,
    )

    print(f"SUCCESS: Release published at {release_url}")


if __name__ == "__main__":
    main()
