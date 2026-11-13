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
#   1. the copy has the same bytes as the binary in <release_dir>,
#   2. cargo wrote a dep-info file next to that binary, <release_dir>/<name>.d,
#      and it names at least one file under <source_root>, and
#   3. every file the dep-info names still exists, and none is newer than the
#      binary.
#
# The dep-info lists the files cargo compiled into the binary: the sources of
# the package and whatever they take in with include_str! or include_bytes!.
# Test modules and license texts are not in it, so a change to one of those
# does not stop a build that cargo rightly left alone. A binary that cargo did
# not have to rebuild is older than the build that finds it, but newer than
# every file it was made from, so it passes. One left in a target folder that
# cargo did not write to this time fails as soon as one of its files has
# changed or been deleted since, a dep-info that names none of this
# checkout's files fails as well, and a copy taken from anywhere else fails
# the byte comparison.
#
# What the check cannot tell is whose build a binary is. With one target
# folder shared by several checkouts, cargo can leave another checkout's
# binary in the release folder and rewrite its dep-info to name this
# checkout's files, when they are all older than that binary; the check then
# passes it. Build a release in a target folder of its own.
#
# Exits 0 when both binaries pass, 1 when one does not, 2 on wrong usage.

set -eu

if [ "$#" -ne 3 ]; then
    echo "usage: $0 <release_dir> <copies_dir> <source_root>" >&2
    exit 2
fi
release_dir=$1
copies_dir=$2
# Cargo writes the paths with symbolic links resolved, so compare with those.
if ! source_root=$(cd "$3" 2>/dev/null && pwd -P); then
    echo "✗ $3 is not a folder" >&2
    exit 2
fi

# The files a make-style dep-info file names, one per line: the words after
# the target's colon in each rule, with "\ " read as a space and a line that
# ends in "\" joined to the next. Comment lines are left out.
dep_files() {
    awk '
        function rule(line,    n, i, word, after_target) {
            if (line ~ /^[ \t]*#/) return
            gsub(/\\ /, "\001", line)
            n = split(line, word, /[ \t]+/)
            after_target = 0
            for (i = 1; i <= n; i++) {
                if (word[i] == "") continue
                if (!after_target) {
                    after_target = (word[i] ~ /:$/)
                    continue
                }
                gsub("\001", " ", word[i])
                print word[i]
            }
        }
        /\\$/ { held = held substr($0, 1, length($0) - 1) " "; next }
        { rule(held $0); held = "" }
        END { if (held != "") rule(held) }
    ' "$1"
}

for binary in ferrispass ferrispass-cli; do
    built="${release_dir}/${binary}"
    if ! cmp -s "${built}" "${copies_dir}/${binary}"; then
        echo "✗ the bundle does not carry the binary this build produced; CARGO_TARGET_DIR points somewhere else" >&2
        echo "  ${copies_dir}/${binary} is not ${built}" >&2
        exit 1
    fi

    dep_info="${built}.d"
    if [ ! -f "${dep_info}" ] || [ ! -r "${dep_info}" ] || ! files=$(dep_files "${dep_info}"); then
        echo "✗ cargo wrote no dep-info for ${binary}; the build did not come from cargo" >&2
        echo "  ${dep_info} is missing or cannot be read" >&2
        exit 1
    fi

    # One argument per file: split at line ends only, and no globbing.
    set -f
    IFS='
'
    # shellcheck disable=SC2086
    set -- ${files}
    unset IFS
    set +f

    ours=false
    for file in "$@"; do
        case ${file} in
            "${source_root}"/*) ours=true ;;
        esac
    done
    if [ "${ours}" != true ]; then
        echo "✗ the bundle does not carry the binary this build produced; CARGO_TARGET_DIR points somewhere else" >&2
        echo "  ${dep_info} names no file under ${source_root}" >&2
        exit 1
    fi

    # Cargo takes a file it compiled that is gone for a change: it rebuilds,
    # or fails without the file. A dep-info that names one is left over.
    for file in "$@"; do
        if [ ! -e "${file}" ]; then
            echo "✗ a source file the binary was built from no longer exists; the build did not rebuild it" >&2
            echo "  ${dep_info} names ${file}, which no longer exists" >&2
            exit 1
        fi
    done

    newer=$(find "$@" -newer "${built}" -print 2>/dev/null | head -n 1)
    if [ -n "${newer}" ]; then
        echo "✗ a source file is newer than the binary; the build did not rebuild it" >&2
        echo "  ${newer} is newer than ${built}" >&2
        exit 1
    fi
done
echo "    ✓ ferrispass and ferrispass-cli are the binaries this build produced"
