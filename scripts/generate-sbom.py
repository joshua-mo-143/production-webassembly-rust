#!/usr/bin/env python3
"""Generate a deterministic CycloneDX 1.6 SBOM from locked Cargo metadata."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import tomllib
import urllib.parse
from pathlib import Path

GUEST_ARTIFACTS = {
    "ch04-guest": "ch04_guest.wasm",
    "ch05-guest": "ch05_guest.wasm",
    "ch06-guest": "ch06_guest.wasm",
    "ch07-guest": "ch07_guest.wasm",
    "ch08-guest": "ch08_guest.wasm",
    "ch10-guest": "ch10_guest.wasm",
    "ch11-plugin-v1": "ch11_plugin_v1.wasm",
    "ch11-plugin-v1-1": "ch11_plugin_v1_1.wasm",
    "ch12-guest": "ch12_guest.wasm",
    "ch13-catalog": "ch13_catalog.wasm",
    "ch13-renderer": "ch13_renderer.wasm",
    "ch14-normalizer": "ch14_normalizer.wasm",
    "ch14-workspace-reader": "ch14_workspace_reader.wasm",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--components-directory", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--timestamp", required=True)
    return parser.parse_args()


def purl(name: str, version: str) -> str:
    return (
        "pkg:cargo/"
        + urllib.parse.quote(name, safe="")
        + "@"
        + urllib.parse.quote(version, safe="")
    )


def included_dependencies(node: dict) -> list[str]:
    dependencies = []
    for dependency in node["deps"]:
        kinds = dependency.get("dep_kinds", [])
        if not kinds or any(kind.get("kind") != "dev" for kind in kinds):
            dependencies.append(dependency["pkg"])
    return dependencies


def main() -> None:
    args = parse_args()
    repository = Path(__file__).resolve().parent.parent
    metadata = json.loads(
        subprocess.check_output(
            [
                "cargo",
                "metadata",
                "--locked",
                "--filter-platform",
                "wasm32-wasip2",
                "--format-version",
                "1",
            ],
            cwd=repository,
            text=True,
        )
    )
    lock_data = tomllib.loads((repository / "Cargo.lock").read_text())
    checksums = {
        (package["name"], package["version"], package.get("source")): package["checksum"]
        for package in lock_data["package"]
        if "checksum" in package
    }

    package_by_id = {package["id"]: package for package in metadata["packages"]}
    node_by_id = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    roots = {}
    for package_name in GUEST_ARTIFACTS:
        matches = [
            package
            for package in metadata["packages"]
            if package["name"] == package_name
            and package["id"] in metadata["workspace_members"]
        ]
        if len(matches) != 1:
            raise SystemExit(
                f"expected one workspace package named {package_name}, found {len(matches)}"
            )
        roots[package_name] = matches[0]["id"]

    included_ids = set(roots.values())
    pending = list(included_ids)
    while pending:
        package_id = pending.pop()
        for dependency in included_dependencies(node_by_id[package_id]):
            if dependency not in included_ids:
                included_ids.add(dependency)
                pending.append(dependency)

    packages = sorted(
        (package_by_id[package_id] for package_id in included_ids),
        key=lambda package: (package["name"], package["version"], package["id"]),
    )
    refs = {package["id"]: purl(package["name"], package["version"]) for package in packages}

    components = []
    for package in packages:
        component = {
            "type": "library",
            "bom-ref": refs[package["id"]],
            "name": package["name"],
            "version": package["version"],
            "purl": refs[package["id"]],
        }
        if package.get("license"):
            component["licenses"] = [{"expression": package["license"]}]
        checksum = checksums.get(
            (package["name"], package["version"], package.get("source"))
        )
        if checksum:
            component["hashes"] = [{"alg": "SHA-256", "content": checksum}]
        components.append(component)

    artifact_refs = {}
    for package_name, artifact_name in GUEST_ARTIFACTS.items():
        artifact_path = args.components_directory / artifact_name
        if not artifact_path.is_file():
            raise SystemExit(f"missing shipped component: {artifact_path}")
        artifact_ref = f"urn:production-webassembly-rust:wasm:{artifact_name}"
        artifact_refs[package_name] = artifact_ref
        components.append(
            {
                "type": "file",
                "bom-ref": artifact_ref,
                "name": artifact_name,
                "version": args.version,
                "hashes": [
                    {
                        "alg": "SHA-256",
                        "content": hashlib.sha256(artifact_path.read_bytes()).hexdigest(),
                    }
                ],
                "properties": [
                    {"name": "cargo:package", "value": package_name},
                    {"name": "wasm:target", "value": "wasm32-wasip2"},
                ],
            }
        )

    dependency_edges = []
    for package_id in sorted(included_ids, key=lambda item: refs[item]):
        dependencies = [
            dependency
            for dependency in included_dependencies(node_by_id[package_id])
            if dependency in included_ids
        ]
        dependency_edges.append(
            {
                "ref": refs[package_id],
                "dependsOn": sorted(refs[dependency] for dependency in dependencies),
            }
        )
    artifact_edges = [
        {
            "ref": artifact_refs[package_name],
            "dependsOn": [refs[roots[package_name]]],
        }
        for package_name in sorted(GUEST_ARTIFACTS)
    ]

    lock_hash = hashlib.sha256((repository / "Cargo.lock").read_bytes()).hexdigest()
    document = {
        "$schema": "https://cyclonedx.org/schema/bom-1.6.schema.json",
        "bomFormat": "CycloneDX",
        "specVersion": "1.6",
        "version": 1,
        "metadata": {
            "timestamp": args.timestamp,
            "tools": {
                "components": [
                    {
                        "type": "application",
                        "name": "scripts/generate-sbom.py",
                        "version": "1",
                    }
                ]
            },
            "component": {
                "type": "application",
                "bom-ref": "production-webassembly-rust",
                "name": "production-webassembly-rust",
                "version": args.version,
                "properties": [
                    {"name": "git:commit", "value": args.commit},
                    {"name": "cargo:lockfile:sha256", "value": lock_hash},
                ],
            },
        },
        "components": components,
        "dependencies": [
            {
                "ref": "production-webassembly-rust",
                "dependsOn": sorted(artifact_refs.values()),
            },
            *artifact_edges,
            *dependency_edges,
        ],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(document, indent=2, ensure_ascii=False, sort_keys=True) + "\n"
    )


if __name__ == "__main__":
    main()
