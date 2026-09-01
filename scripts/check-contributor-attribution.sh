#!/usr/bin/env bash
set -euo pipefail

base="${1:?base commit is required}"
head="${2:?head commit is required}"
event_name="${3:?event name is required}"

case "$event_name" in
  push|pull_request) ;;
  *)
    printf 'ERROR: unsupported attribution event: %s\n' "$event_name" >&2
    exit 2
    ;;
esac

canonical_identities="${CANONICAL_AUTHOR_IDENTITIES:-ZEKRITI Tarek <194770978+CHECKUPAUTO@users.noreply.github.com>}"

if [[ "$base" =~ ^0+$ ]]; then
  range="$head"
else
  range="$base..$head"
fi

is_github_synthetic_merge() {
  local sha="$1"
  local parents committer_name committer_email subject

  [[ "$event_name" == "push" && "$sha" == "$head" ]] || return 1
  parents="$(git show -s --format='%P' "$sha")"
  [[ "$(wc -w <<<"$parents")" -ge 2 ]] || return 1
  committer_name="$(git show -s --format='%cn' "$sha")"
  committer_email="$(git show -s --format='%ce' "$sha")"
  subject="$(git show -s --format='%s' "$sha")"

  [[ "$committer_name" == "GitHub" ]] || return 1
  [[ "$committer_email" == "noreply@github.com" ]] || return 1
  [[ "$subject" =~ ^Merge\ pull\ request\ \#[0-9]+\ from\  ]]
}

is_authorized_identity() {
  local identity="$1 <$2>"
  local allowed

  while IFS= read -r allowed; do
    [[ -n "$allowed" ]] || continue
    [[ "$identity" == "$allowed" ]] && return 0
  done <<< "$canonical_identities"

  return 1
}

failed=0
while read -r sha; do
  [[ -n "$sha" ]] || continue

  if is_github_synthetic_merge "$sha"; then
    printf 'Skipping GitHub synthetic merge commit %s; source commits remain checked.\n' "$sha"
    continue
  fi

  author_name="$(git show -s --format='%an' "$sha")"
  author_email="$(git show -s --format='%ae' "$sha")"

  if ! is_authorized_identity "$author_name" "$author_email"; then
    printf 'ERROR: commit %s has unauthorized author: %s <%s>\n' "$sha" "$author_name" "$author_email" >&2
    failed=1
  fi

  if git show -s --format='%B' "$sha" | grep -qiE '^[[:space:]]*co-authored-by:[[:space:]]*'; then
    printf 'ERROR: commit %s contains forbidden Co-authored-by attribution\n' "$sha" >&2
    failed=1
  fi
done < <(git rev-list --reverse "$range")

(( failed == 0 )) || exit 1
printf 'Canonical attribution policy satisfied.\n'
