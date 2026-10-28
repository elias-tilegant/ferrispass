#!/usr/bin/env bash
#
# Points the pre-release channel at the highest version released so far.
#
#   scripts/update-channel.sh [owner/repo]
#
# kanal-vorab is a permanent pre-release that holds one pair of files,
# update.json and update.json.minisig, copied from a v* release. This script
# finds the highest version, by SemVer precedence, among the published v*
# releases that carry both files, fully uploaded, and makes kanal-vorab hold
# exactly that pair, unless the channel already holds a higher version: the
# channel only moves forward. A v* release whose tag is not a version is
# skipped with a warning.
#
# Because it looks at every release, not only the one just published, any
# run puts right what an earlier run left behind: a run that was cancelled
# while it waited, a release that was signed again, or an upload that broke
# off. gh release upload --clobber deletes the old files before it uploads
# the new ones, so a broken-off upload leaves the channel empty or holding
# one of the two files. A lone manifest still says its version, and the
# channel moves on from there as from a pair. A lone signature says nothing,
# but every pair the channel held came from a release, so the highest
# release is not lower than it, and its pair goes in.
#
# It fails, and leaves the channel as it is, when kanal-vorab is missing, is
# not a pre-release (a full release could become "latest" and hand test
# builds to everyone), holds any other file, holds a lone manifest above
# every release, or when a manifest or the channel's version cannot be read,
# or when the manifest a release carries names another version than its tag.
#
# The release workflow runs it after publishing; it also runs by hand, with
# gh logged in to an account that may edit releases. Needs gh, jq, python3.

set -euo pipefail

REPO="${1:-${GITHUB_REPOSITORY:?pass owner/repo or set GITHUB_REPOSITORY}}"
CHANNEL=kanal-vorab
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

# Prints higher, equal or lower: where $1 stands relative to $2. A version it
# cannot read stops the script (exit 2), here and wherever the result is
# assigned.
compare() {
    python3 -I "${HERE}/semver-compare.py" "$1" "$2"
}

fail() {
    echo "::error::$*"
    exit 1
}

# Succeeds when $1 is a SemVer version and fails when it is not. Anything
# else, such as a missing python3, stops the script.
is_version() {
    local status=0
    compare "$1" "$1" >/dev/null 2>&1 || status=$?
    case "${status}" in
        0) return 0 ;;
        2) return 1 ;;
        *) fail "cannot compare versions (exit ${status})" ;;
    esac
}

# What the channel holds now, counting only files that finished uploading.
# gh fails, and the script with it, when the release does not exist.
state="$(gh release view "${CHANNEL}" --repo "${REPO}" --json isPrerelease,assets --jq '
    "\(.isPrerelease) \([.assets[] | select(.state == "uploaded") | .name] | sort | join(" "))"')"
prerelease="${state%% *}"
held="${state#* }"
if [ "${prerelease}" != true ]; then
    fail "${CHANNEL} is not a pre-release, so it could become latest; mark it as a pre-release by hand"
fi
current=""  # the version the channel holds, when it is known
complete=false
mkdir -p "${WORK}/channel"
case "${held}" in
    "") ;;
    "update.json update.json.minisig")
        gh release download "${CHANNEL}" --repo "${REPO}" \
            --pattern update.json --pattern update.json.minisig --dir "${WORK}/channel"
        current="$(jq -er .version "${WORK}/channel/update.json")"
        complete=true
        ;;
    "update.json")
        echo "::warning::${CHANNEL} holds update.json without its signature: an upload broke off"
        gh release download "${CHANNEL}" --repo "${REPO}" --pattern update.json \
            --dir "${WORK}/channel"
        current="$(jq -er .version "${WORK}/channel/update.json")"
        ;;
    "update.json.minisig")
        echo "::warning::${CHANNEL} holds update.json.minisig without its manifest: an upload broke off"
        ;;
    *)
        fail "${CHANNEL} holds '${held}', not update.json and update.json.minisig; repair it by hand"
        ;;
esac

# The published v* releases that carry both files, fully uploaded, and the
# highest of them.
tags="$(gh api --paginate "repos/${REPO}/releases?per_page=100" --jq '
    .[] | select(.draft | not) | select(.tag_name | startswith("v"))
        | select(any(.assets[]; .name == "update.json" and .state == "uploaded"))
        | select(any(.assets[]; .name == "update.json.minisig" and .state == "uploaded"))
        | .tag_name')"
best=""
while IFS= read -r tag; do
    [ -n "${tag}" ] || continue
    version="${tag#v}"
    if ! is_version "${version}"; then
        echo "::warning::skipping release ${tag}: its tag is not a version"
        continue
    fi
    order="$(compare "${version}" "${best:-${version}}")"
    if [ -z "${best}" ] || [ "${order}" = higher ]; then
        best="${version}"
    fi
done <<< "${tags}"
if [ -z "${best}" ]; then
    fail "no published v* release carries update.json and update.json.minisig"
fi

order=higher
if [ -n "${current}" ]; then
    order="$(compare "${best}" "${current}")"
fi
if [ "${order}" = lower ]; then
    if [ "${complete}" != true ]; then
        fail "${CHANNEL} holds a broken-off upload of ${current}, above the highest release with a manifest, ${best}; repair it by hand"
    fi
    echo "::notice::${CHANNEL} stays at ${current}, above the highest release with a manifest, ${best}"
    exit 0
fi

mkdir -p "${WORK}/next"
gh release download "v${best}" --repo "${REPO}" \
    --pattern update.json --pattern update.json.minisig --dir "${WORK}/next"
manifest="$(jq -er .version "${WORK}/next/update.json")"
if [ "${manifest}" != "${best}" ]; then
    fail "the manifest attached to v${best} says ${manifest}"
fi
if [ "${complete}" = true ] && [ "${order}" = equal ] \
    && cmp -s "${WORK}/next/update.json" "${WORK}/channel/update.json" \
    && cmp -s "${WORK}/next/update.json.minisig" "${WORK}/channel/update.json.minisig"; then
    echo "::notice::${CHANNEL} stays at ${current}, the highest release with a manifest"
    exit 0
fi
gh release upload "${CHANNEL}" "${WORK}/next/update.json" "${WORK}/next/update.json.minisig" \
    --clobber --repo "${REPO}"
echo "${CHANNEL} now holds ${best}${current:+ (was ${current})}"
