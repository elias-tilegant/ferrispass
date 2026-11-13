#!/bin/sh
#
# Stops a release build from packaging binaries it did not produce.
#
# Usage: check-bundle-binaries.sh <release_dir> <copies_dir> <source_root>
#
# scripts/build-mac.sh runs this right after it copies ferrispass and
# ferrispass-cli out of cargo's release folder, <release_dir>, into
# <copies_dir>, the folder the bundle takes them from. For each of the two it
# requires that
#
#   1. the copy has the same bytes as the binary in <release_dir>, and
#   2. no file under src/, locales/ or assets/ of <source_root>, and neither
#      Cargo.toml nor Cargo.lock, is newer than the binary in <release_dir>.
#
# A binary that cargo did not have to rebuild is older than the build that
# finds it, but still newer than every source, so it passes. One that sits in
# a target folder cargo did not write to this time fails the second test as
# soon as any source has changed since, and a copy taken from anywhere else
# fails the first.
#
# Exits 0 when both binaries pass, 1 when one does not, 2 on wrong usage.

set -eu

if [ "$#" -ne 3 ]; then
    echo "usage: $0 <release_dir> <copies_dir> <source_root>" >&2
    exit 2
fi
release_dir=$1
copies_dir=$2
source_root=$3

for source in src locales assets Cargo.toml Cargo.lock; do
    if [ ! -e "${source_root}/${source}" ]; then
        echo "✗ ${source_root}/${source} not found" >&2
        exit 2
    fi
done

for binary in ferrispass ferrispass-cli; do
    if ! cmp -s "${release_dir}/${binary}" "${copies_dir}/${binary}"; then
        echo "✗ the bundle does not carry the binary this build produced; CARGO_TARGET_DIR points somewhere else" >&2
        echo "  ${copies_dir}/${binary} is not ${release_dir}/${binary}" >&2
        exit 1
    fi
    newer=$(find "${source_root}/src" "${source_root}/locales" "${source_root}/assets" \
        "${source_root}/Cargo.toml" "${source_root}/Cargo.lock" \
        -type f -newer "${release_dir}/${binary}" -print | head -n 1)
    if [ -n "${newer}" ]; then
        echo "✗ a source file is newer than the binary; the build did not rebuild it" >&2
        echo "  ${newer} is newer than ${release_dir}/${binary}" >&2
        exit 1
    fi
done
echo "    ✓ ferrispass and ferrispass-cli are the binaries this build produced"
