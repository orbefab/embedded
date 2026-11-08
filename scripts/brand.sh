#!/usr/bin/env bash
# The brand copy in packages/brand/website: files copied unchanged from the website repo at the
# commit in packages/brand/website.commit, each listed with its SHA-256 in
# packages/brand/website.sha256.
#
#   scripts/brand.sh check [WEBSITE_CHECKOUT]
#     Fails unless every file under packages/brand/website is listed and has its listed hash.
#     With a checkout of the website repo, it also fails unless each file has the bytes it has
#     at that commit there.
#   scripts/brand.sh copy WEBSITE_CHECKOUT [PATH...]
#     Copies each PATH, by default every listed one, from the checkout's HEAD commit, then
#     rewrites the commit and the list. It reads the commit with git, so the checkout's
#     uncommitted changes are never copied and the checkout is never written.
set -euo pipefail
cd "$(dirname "$0")/../packages/brand"

sha() {
  if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi
}

listed() {
  sed 's/^[0-9a-f]\{64\}  //' website.sha256
}

need_commit() {
  local checkout=$1 commit=$2
  if ! git -C "$checkout" cat-file -e "$commit^{commit}" 2>/dev/null; then
    echo "brand: $checkout has no commit $commit. Fetch the website repo there first." >&2
    exit 1
  fi
}

check() {
  local present unlisted missing failed=0
  present=$(cd website && find . -type f | sed 's|^\./||' | LC_ALL=C sort)
  unlisted=$(LC_ALL=C comm -23 <(echo "$present") <(listed | LC_ALL=C sort))
  missing=$(LC_ALL=C comm -13 <(echo "$present") <(listed | LC_ALL=C sort))
  if [[ -n $unlisted ]]; then
    echo "brand: files in packages/brand/website that website.sha256 does not list:" >&2
    echo "$unlisted" | sed 's/^/  /' >&2
    failed=1
  fi
  if [[ -n $missing ]]; then
    echo "brand: files website.sha256 lists that packages/brand/website lacks:" >&2
    echo "$missing" | sed 's/^/  /' >&2
    failed=1
  fi
  if ! (cd website && sha --check --quiet ../website.sha256 >&2); then
    echo "brand: a copied file differs from its listed hash. Copy it again with scripts/brand.sh copy." >&2
    failed=1
  fi
  if [[ $# -eq 1 ]]; then
    local checkout=$1 commit path
    commit=$(<website.commit)
    need_commit "$checkout" "$commit"
    while IFS= read -r path; do
      if ! git -C "$checkout" show "$commit:$path" 2>/dev/null | cmp -s - "website/$path"; then
        echo "brand: website/$path is not the website's $path at $commit" >&2
        failed=1
      fi
    done < <(listed)
  fi
  if [[ $failed -ne 0 ]]; then exit 1; fi
  echo "brand: $(listed | wc -l | tr -d ' ') files match website.sha256${1:+ and the website at $(<website.commit)}"
}

copy() {
  local checkout=$1 commit path
  shift
  local paths=("$@")
  if [[ ${#paths[@]} -eq 0 ]]; then mapfile -t paths < <(listed); fi
  commit=$(git -C "$checkout" rev-parse HEAD)
  need_commit "$checkout" "$commit"
  rm -rf website
  for path in "${paths[@]}"; do
    mkdir -p "website/$(dirname "$path")"
    git -C "$checkout" show "$commit:$path" >"website/$path"
  done
  echo "$commit" >website.commit
  (cd website && printf '%s\n' "${paths[@]}" | LC_ALL=C sort | while IFS= read -r path; do sha "$path"; done) >website.sha256
  echo "brand: copied ${#paths[@]} files from the website at $commit"
}

case "${1:-}" in
  check) [[ $# -le 2 ]] || { echo "usage: scripts/brand.sh check [WEBSITE_CHECKOUT]" >&2; exit 2; }; shift; check "$@" ;;
  copy) [[ $# -ge 2 ]] || { echo "usage: scripts/brand.sh copy WEBSITE_CHECKOUT [PATH...]" >&2; exit 2; }; shift; copy "$@" ;;
  *) echo "usage: scripts/brand.sh check [WEBSITE_CHECKOUT] | copy WEBSITE_CHECKOUT [PATH...]" >&2; exit 2 ;;
esac
