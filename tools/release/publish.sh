#!/usr/bin/env bash
# Publishes a release (D266): pushes main, tags its commit "Release VERSION:"
# (an annotated tag vVERSION, "Isocron VERSION"), pushes the tag, and makes
# the release on GitHub, "Isocron VERSION", with the archives of package.sh
# and its notes:
#   publish.sh VERSION DIR [--dry-run]
# DIR: the output of package.sh (isocron-VERSION-x86_64-linux.tar.gz,
# isocron-VERSION-x86_64-windows.zip, SHA256SUMS) and notes.md, the text of
# the release. Checked first: main checked out with nothing to commit, the
# commit of the release with that version in Cargo.toml and a section in
# CHANGELOG.md, the sums of the archives, `gh` logged in, no tag or release
# of that version elsewhere. With --dry-run, the checks only, and the
# commands it would run.
set -euo pipefail
usage() { echo "usage: publish.sh VERSION DIR [--dry-run]" >&2; exit 2; }
[ $# -ge 2 ] || usage
v=$1
dir=$(realpath "$2")
dry=0
[ "${3:-}" = "--dry-run" ] && dry=1
[ $# -le 3 ] || usage
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
fail() { echo "publish.sh: $*" >&2; exit 1; }
run() {
    echo "+ $*"
    [ $dry = 1 ] || "$@"
}

# the tree and the commit of the release
[ "$(git branch --show-current)" = main ] || fail "not on main"
git diff --quiet && git diff --cached --quiet || fail "changes not committed"
commit=$(git log --format=%H -1 --grep="^Release $v:" main)
[ -n "$commit" ] || fail "no commit \"Release $v: ...\" on main"
git show "$commit:Cargo.toml" | grep -qx "version = \"$v\"" || fail "Cargo.toml of $commit is not at version $v"
git show "$commit:CHANGELOG.md" | grep -q "^## $v " || fail "CHANGELOG.md of $commit has no section $v"

# the files of the release
assets=("$dir/isocron-$v-x86_64-linux.tar.gz" "$dir/isocron-$v-x86_64-windows.zip" "$dir/SHA256SUMS")
for f in "${assets[@]}" "$dir/notes.md"; do
    [ -s "$f" ] || fail "missing: $f"
done
(cd "$dir" && sha256sum --check --quiet SHA256SUMS) || fail "the sums of $dir do not match"

# GitHub, and what is already there
gh auth status > /dev/null 2>&1 || fail "gh is not logged in (gh auth login)"
git fetch --quiet origin main --tags
git merge-base --is-ancestor origin/main main || fail "main is not ahead of origin/main"
if git rev-parse -q --verify "refs/tags/v$v" > /dev/null; then
    [ "$(git rev-parse "v$v^{commit}")" = "$commit" ] || fail "the tag v$v exists on another commit"
    tag_made=1
else
    tag_made=0
fi
if gh release view "v$v" > /dev/null 2>&1; then
    fail "the release v$v already exists on GitHub"
fi

echo "release $v: commit $(git log -1 --format='%h %s' "$commit" | cut -c1-72)"
echo "  $(git rev-list --count origin/main..main) commit(s) to push, the files of $dir"
run git push origin main
[ $tag_made = 1 ] || run git tag -a "v$v" -m "Isocron $v" "$commit"
run git push origin "v$v"
run gh release create "v$v" "${assets[@]}" --title "Isocron $v" --notes-file "$dir/notes.md" --verify-tag
[ $dry = 1 ] && echo "(dry run: nothing pushed or published)"
exit 0
