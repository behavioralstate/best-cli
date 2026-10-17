#!/usr/bin/env bash
# Cuts a release of best-cli: the version in Cargo.toml is the one source of truth, the tag `v<version>` on a commit
# of main is what publishes it (.github/workflows/release.yml builds, attests and publishes the binaries).
#
#   scripts/release.sh <version> -m "<what this release is, one or two sentences>"
#   scripts/release.sh 0.2.0 -m "Follows the live stream with Last-Event-ID across reconnects."
#
# It refuses to run anywhere but a clean main that is level with origin, bumps the version, refreshes the lockfile,
# commits "Release <version>" with the message as its body (that body heads the release notes), tags it with the same
# message, and pushes main and the tag. The message is required: a release says what it is.
set -euo pipefail

usage() { sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }

version=""; message=""
while [ $# -gt 0 ]; do
  case "$1" in
    -m) [ $# -ge 2 ] || { echo "-m needs the message"; exit 2; }; message="$2"; shift 2 ;;
    -h|--help) usage ;;
    *) if [ -z "$version" ]; then version="$1"; shift; else echo "unexpected argument $1"; usage; fi ;;
  esac
done
[ -n "$version" ] || usage
[ -n "$message" ] || { echo "a release says what it is: pass -m \"...\""; exit 2; }
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || { echo "$version is not a semver version (MAJOR.MINOR.PATCH, optional -pre)"; exit 2; }

cd "$(git rev-parse --show-toplevel)"
[ "$(git rev-parse --abbrev-ref HEAD)" = main ] || { echo "releases are cut from main"; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "the tree is not clean"; exit 1; }
git fetch -q --tags origin main
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || { echo "main is not level with origin/main: push (through the review gate) or pull first"; exit 1; }
git rev-parse -q --verify "refs/tags/v$version" > /dev/null && { echo "v$version already exists"; exit 1; }
[ -z "$(git ls-remote --tags origin "refs/tags/v$version")" ] || { echo "v$version already exists on origin"; exit 1; }
current=$(grep -m1 -E '^version = "' Cargo.toml | sed -E 's/version = "(.*)"/\1/')
# Cargo.toml may already carry the version (the first release of a crate that was born at it): the tag checks above
# say it was never released, so it is bumped to nothing and the release commit is an empty one that carries the message.
same=""; [ "$current" != "$version" ] || same=1
latest=$(git tag --list 'v*' --sort=-version:refname | head -1)
[ -z "$latest" ] || echo "latest release: $latest"

# The one line under [package]. Perl, because sed differs between GNU and BSD. Until the commit is made, a failure
# puts the tree back as it was.
trap 'git checkout -q -- Cargo.toml Cargo.lock; echo "nothing changed"' ERR
[ -n "$same" ] || CURRENT="$current" VERSION="$version" perl -pi -e 'if (!$done && s/^version = "\Q$ENV{CURRENT}\E"$/version = "$ENV{VERSION}"/) { $done = 1 }' Cargo.toml
grep -q "^version = \"$version\"$" Cargo.toml || { echo "could not set the version in Cargo.toml"; false; }
cargo update --package best-cli --offline -q 2>/dev/null || cargo update --package best-cli -q
cargo build --release --locked -q
[ "$(./target/release/best --version)" = "best $version" ] || { echo "the binary does not say $version"; false; }

git add Cargo.toml Cargo.lock
git commit -q ${same:+--allow-empty} -m "Release $version" -m "$message"
git tag -a "v$version" -m "$message"
trap 'echo "the push failed: main and v'"$version"' are only local. To try again: fix the cause, then git push origin main && git push origin v'"$version"'. To undo: git tag -d v'"$version"' && git reset --hard origin/main"' ERR
git push origin main
git push origin "v$version"
trap - ERR
echo "v$version pushed: https://github.com/behavioralstate/best-cli/actions/workflows/release.yml"
