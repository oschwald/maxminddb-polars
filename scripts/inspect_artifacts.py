#!/usr/bin/env python3
"""Validate the contents and metadata of release artifacts."""

from __future__ import annotations

import argparse
import email.parser
import tarfile
import zipfile
from collections.abc import Callable, Iterable
from email.message import Message
from pathlib import Path, PurePosixPath

PROJECT_NAME = "maxminddb-polars"
PACKAGE = PurePosixPath("maxminddb_polars")


def _require(
    names: Iterable[str], predicate: Callable[[str], bool], description: str
) -> None:
    if not any(predicate(name) for name in names):
        raise ValueError(f"missing {description}")


def _reject_test_databases(names: Iterable[str]) -> None:
    databases = [name for name in names if name.lower().endswith(".mmdb")]
    if databases:
        raise ValueError(f"artifact contains test databases: {databases}")


def _validate_metadata(metadata: Message, expected_version: str | None) -> None:
    if metadata["Name"] != PROJECT_NAME:
        raise ValueError(f"unexpected project name: {metadata['Name']!r}")
    if expected_version is not None and metadata["Version"] != expected_version:
        raise ValueError(f"unexpected project version: {metadata['Version']!r}")
    if metadata["Requires-Python"] != ">=3.10":
        raise ValueError(
            f"unexpected Python requirement: {metadata['Requires-Python']!r}"
        )
    requirements = metadata.get_all("Requires-Dist", [])
    if not any(requirement.startswith("polars") for requirement in requirements):
        raise ValueError(f"missing Polars requirement: {requirements}")


def inspect_wheel(path: Path, expected_version: str | None) -> None:
    if "-cp310-abi3-" not in path.name:
        raise ValueError(f"wheel does not have the expected abi3 tag: {path.name}")
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        _reject_test_databases(names)
        _require(
            names,
            lambda name: PurePosixPath(name) == PACKAGE / "__init__.py",
            "maxminddb_polars/__init__.py",
        )
        _require(
            names,
            lambda name: (
                PurePosixPath(name).parent == PACKAGE
                and PurePosixPath(name).name.startswith("_maxminddb_polars")
                and PurePosixPath(name).suffix in {".so", ".pyd", ".dylib"}
            ),
            "native extension inside maxminddb_polars",
        )
        _require(
            names,
            lambda name: PurePosixPath(name).name == "LICENSE",
            "license file",
        )
        _require(
            names,
            lambda name: ".dist-info/sboms/" in name and name.endswith(".json"),
            "CycloneDX SBOM",
        )

        metadata_names = [
            name for name in names if name.endswith(".dist-info/METADATA")
        ]
        if len(metadata_names) != 1:
            raise ValueError(f"expected one METADATA file, found {metadata_names}")
        metadata = email.parser.BytesParser().parsebytes(
            archive.read(metadata_names[0])
        )

    _validate_metadata(metadata, expected_version)


def inspect_sdist(path: Path, expected_version: str | None) -> None:
    with tarfile.open(path, mode="r:gz") as archive:
        names = [member.name for member in archive.getmembers()]
        metadata_members = [
            member
            for member in archive.getmembers()
            if member.name.endswith("/PKG-INFO")
        ]
        if len(metadata_members) != 1:
            raise ValueError(f"expected one PKG-INFO file, found {metadata_members}")
        metadata_file = archive.extractfile(metadata_members[0])
        if metadata_file is None:
            raise ValueError("could not read PKG-INFO")
        metadata = email.parser.BytesParser().parsebytes(metadata_file.read())

    _reject_test_databases(names)
    _validate_metadata(metadata, expected_version)
    required_paths = {
        "Cargo.lock",
        "Cargo.toml",
        "LICENSE",
        "maxminddb_polars/__init__.py",
        "pyproject.toml",
        "src/lib.rs",
        "uv.lock",
    }
    stripped_names = {
        "/".join(PurePosixPath(name).parts[1:]) for name in names if "/" in name
    }
    missing = sorted(required_paths - stripped_names)
    if missing:
        raise ValueError(f"sdist is missing required files: {missing}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--expected-version")
    parser.add_argument("artifacts", type=Path, nargs="+")
    args = parser.parse_args()

    for path in args.artifacts:
        if path.suffix == ".whl":
            inspect_wheel(path, args.expected_version)
        elif path.name.endswith(".tar.gz"):
            inspect_sdist(path, args.expected_version)
        else:
            raise ValueError(f"unsupported artifact: {path}")
        print(f"validated {path}")


if __name__ == "__main__":
    main()
