#!/usr/bin/env bash
#
# Points the pre-release channel at the highest version released so far.
#
#   scripts/update-channel.sh [owner/repo]
#
# kanal-vorab is a permanent pre-release that holds exactly one pair of files,
# update.json and update.json.minisig, copied from a v* release. This script
# finds the highest version, by SemVer precedence, among the published v*
# releases that carry both files, and makes kanal-vorab hold exactly that
# pair, unless the channel already holds a higher version: the channel only
# moves forward. Because it looks at every release, not only the one just
# published, any run puts right a channel that an earlier run left behind:
# one that was cancelled while it waited, an upload that broke off between
# the two files, or a pair the release has since been re-signed with.
#
# It fails, and leaves the channel as it is, when kanal-vorab is missing, is
# not a pre-release (a full release could become "latest" and hand test
# builds to everyone), holds anything but nothing or exactly that pair, or
# when a manifest or a version cannot be read.
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

# What the channel holds now. gh fails, and the script with it, when the
# release does not exist.
state="$(gh release view "${CHANNEL}" --repo "${REPO}" --json isPrerelease,assets \
    --jq '"\(.isPrerelease) \([.assets[].name] | sort | join(" "))"')"
prerelease="${state%% *}"
held="${state#* }"
if [ "${prerelease}" != true ]; then
    fail "${CHANNEL} is not a pre-release, so it could become latest; mark it as a pre-release by hand"
fi
case "${held}" in
    "")
        current=""
        ;;
    "update.json update.json.minisig")
        mkdir -p "${WORK}/channel"
        gh release download "${CHANNEL}" --repo "${REPO}" \
            --pattern update.json --pattern update.json.minisig --dir "${WORK}/channel"
        current="$(jq -er .version "${WORK}/channel/update.json")"
        ;;
    *)
        fail "${CHANNEL} holds '${held}', not update.json and update.json.minisig; repair it by hand"
        ;;
esac

# The published v* releases that carry both files, and the highest of them.
tags="$(gh api --paginate "repos/${REPO}/releases?per_page=100" --jq '
    .[] | select(.draft | not) | select(.tag_name | startswith("v"))
        | select(any(.assets[]; .name == "update.json"))
        | select(any(.assets[]; .name == "update.json.minisig"))
        | .tag_name')"
best=""
while IFS= read -r tag; do
    [ -n "${tag}" ] || continue
    version="${tag#v}"
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
if [ "${order}" = equal ] \
    && cmp -s "${WORK}/next/update.json" "${WORK}/channel/update.json" \
    && cmp -s "${WORK}/next/update.json.minisig" "${WORK}/channel/update.json.minisig"; then
    echo "::notice::${CHANNEL} stays at ${current}, the highest release with a manifest"
    exit 0
fi
gh release upload "${CHANNEL}" "${WORK}/next/update.json" "${WORK}/next/update.json.minisig" \
    --clobber --repo "${REPO}"
echo "${CHANNEL} now holds ${best}${current:+ (was ${current})}"
