#!/usr/bin/env python3
"""
Queries the authoritative Fedora Bodhi REST API for currently active stable releases
and generates the dynamic rolling 3-version matrix [N, N-1, N-2].
"""

import json
import os
import sys
import urllib.request

BODHI_RELEASES_URL = "https://bodhi.fedoraproject.org/releases/?state=current"


def fetch_bodhi_releases(url: str = BODHI_RELEASES_URL) -> dict:
    req = urllib.request.Request(
        url,
        headers={"User-Agent": "plasma-sip-client-release-ci/1.0"}
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        if resp.status != 200:
            raise RuntimeError(f"HTTP status {resp.status} from {url}")
        raw = resp.read().decode("utf-8")
        return json.loads(raw)


def resolve_matrix(data: dict):
    releases = data.get("releases", [])
    if not releases:
        raise ValueError("Empty releases list returned from Bodhi API")

    fedora_stable_versions = []
    for r in releases:
        # Match official Fedora Linux stable releases:
        # id_prefix must be 'FEDORA' (excludes EPEL)
        # state must be 'current' (excludes pending/branched/testing)
        # name must strictly match F<number> (excludes Containers and Flatpaks)
        if r.get("id_prefix") == "FEDORA" and r.get("state") == "current":
            name = r.get("name", "")
            if name.startswith("F") and name[1:].isdigit():
                v_str = r.get("version")
                if v_str and v_str.isdigit():
                    fedora_stable_versions.append(int(v_str))

    if not fedora_stable_versions:
        raise ValueError("No active stable Fedora Linux releases found in response")

    latest_n = max(fedora_stable_versions)
    if latest_n < 35:
        raise ValueError(f"Sanity check failed: latest stable Fedora version {latest_n} is unexpectedly low")

    # Rolling 3-version support policy: latest stable N, N-1, and N-2
    target_versions = [str(latest_n), str(latest_n - 1), str(latest_n - 2)]
    return latest_n, target_versions


def main():
    print(f"Connecting to authoritative Bodhi API: {BODHI_RELEASES_URL}...")
    try:
        data = fetch_bodhi_releases(BODHI_RELEASES_URL)
        latest_n, matrix = resolve_matrix(data)
    except Exception as e:
        sys.exit(f"Fedora discovery failed: {e}")

    matrix_json = json.dumps(matrix)
    print(f"Latest stable Fedora release (N): {latest_n}")
    print(f"Resolved build matrix [N, N-1, N-2]: {matrix_json}")

    github_output = os.environ.get("GITHUB_OUTPUT")
    if github_output:
        with open(github_output, "a") as f:
            f.write(f"matrix={matrix_json}\n")
            f.write(f"latest_stable={latest_n}\n")
            f.write(f"source_url={BODHI_RELEASES_URL}\n")


if __name__ == "__main__":
    main()
