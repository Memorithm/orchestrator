from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if text.count(old) != 1:
        raise SystemExit(f"expected exactly one match for {label}, found {text.count(old)}")
    return text.replace(old, new, 1)


git_path = Path("scripts/git")
git_text = git_path.read_text()
git_text = replace_once(
    git_text,
    '''if ! grep -Eqi 'non-fast-forward|fetch first|rejected.*HEAD|failed to push some refs' <<<"$push_output"; then''',
    '''if ! grep -Eqi 'non-fast-forward|fetch first' <<<"$push_output"; then''',
    "push race signature",
)
git_text = git_text.replace(
    '''# Retry only the race we know how to reconcile safely. Authentication,
# permissions, hooks, network errors, and every other push failure remain
# fail-closed and are returned to the parent unchanged.''',
    '''# Retry only explicit non-fast-forward/fetch-first races. Generic Git
# footers such as "failed to push some refs" and remote rejections also occur
# for permission/hooks/policy failures and must never trigger reconciliation.
# Authentication, permissions, hooks, network errors, and every other failure
# remain fail-closed and are returned to the parent unchanged.''',
    1,
)
git_path.write_text(git_text)

selftest_path = Path("scripts/selftest-runtime.sh")
selftest = selftest_path.read_text()
anchor = '''printf 'PASS parent-git canonical identity\\n'

# Restore agent-boundary fake Git for any later checks.
export ORCHESTRATOR_REAL_GIT="$TMP_ROOT/fake-git"
'''
if selftest.count(anchor) != 1:
    raise SystemExit("parent git selftest anchor changed")
addition = '''printf 'PASS parent-git canonical identity\\n'

# ---------------------------------------------------------------------------
# Parent push-race classifier: permission/policy rejection must fail closed;
# only an explicit fetch-first/non-fast-forward signature may reconcile.
# ---------------------------------------------------------------------------
cat >"$TMP_ROOT/fake-push-git" <<'EOF'
#!/usr/bin/env bash
set -u
[[ "${GIT_AUTHOR_NAME:-}" == 'ZEKRITI Tarek' ]] || exit 61
[[ "${GIT_AUTHOR_EMAIL:-}" == '194770978+CHECKUPAUTO@users.noreply.github.com' ]] || exit 62
[[ "${GIT_COMMITTER_NAME:-}" == 'ZEKRITI Tarek' ]] || exit 63
[[ "${GIT_COMMITTER_EMAIL:-}" == '194770978+CHECKUPAUTO@users.noreply.github.com' ]] || exit 64
printf '%s\\n' "$*" >>"${SELFTEST_PUSH_TRACE:?}"
mode="${SELFTEST_PUSH_MODE:?}"
case "$mode:$*" in
  'permission:push origin HEAD')
    printf '%s\\n' 'remote: Permission denied by repository policy' >&2
    printf '%s\\n' '! [remote rejected] HEAD -> feature (permission denied)' >&2
    printf '%s\\n' 'error: failed to push some refs to origin' >&2
    exit 1
    ;;
  'race:push origin HEAD')
    printf '%s\\n' '! [rejected] HEAD -> feature (fetch first)' >&2
    printf '%s\\n' 'error: failed to push some refs to origin' >&2
    exit 1
    ;;
  'race:branch --show-current')
    printf '%s\\n' 'feature'
    ;;
  'race:rev-parse HEAD')
    printf '%s\\n' 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
    ;;
  'race:update-ref refs/orchestrator/recovery/feature aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa') ;;
  'race:fetch origin feature') ;;
  'race:rev-parse --verify refs/remotes/origin/feature')
    printf '%s\\n' 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
    ;;
  'race:merge-base --is-ancestor aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa refs/remotes/origin/feature') ;;
  'race:update-ref -d refs/orchestrator/recovery/feature') ;;
  *)
    printf 'unexpected fake push git command in %s mode: %s\\n' "$mode" "$*" >&2
    exit 95
    ;;
esac
EOF
chmod 700 "$TMP_ROOT/fake-push-git"
export ORCHESTRATOR_REAL_GIT="$TMP_ROOT/fake-push-git"
export SELFTEST_PUSH_TRACE="$TMP_ROOT/push.trace"

: >"$SELFTEST_PUSH_TRACE"
if SELFTEST_PUSH_MODE=permission bash "$ROOT/scripts/git" push origin HEAD >/dev/null 2>&1; then
  fail 'permission rejection was incorrectly treated as recoverable push race'
fi
if grep -Fq 'branch --show-current' "$SELFTEST_PUSH_TRACE"; then
  fail 'permission rejection entered push-race reconciliation'
fi
[[ "$(wc -l < "$SELFTEST_PUSH_TRACE")" -eq 1 ]] || fail 'permission rejection executed extra Git commands'
printf 'PASS parent-git permission rejection fails closed\\n'

: >"$SELFTEST_PUSH_TRACE"
SELFTEST_PUSH_MODE=race bash "$ROOT/scripts/git" push origin HEAD >/dev/null 2>&1 || \
  fail 'explicit fetch-first race did not enter safe recovery'
grep -Fxq 'branch --show-current' "$SELFTEST_PUSH_TRACE" || \
  fail 'explicit fetch-first race did not inspect the current branch'
grep -Fxq 'fetch origin feature' "$SELFTEST_PUSH_TRACE" || \
  fail 'explicit fetch-first race did not fetch the raced branch'
printf 'PASS parent-git explicit race recovery classification\\n'

# Restore agent-boundary fake Git for any later checks.
export ORCHESTRATOR_REAL_GIT="$TMP_ROOT/fake-git"
'''
selftest = selftest.replace(anchor, addition, 1)
selftest_path.write_text(selftest)
