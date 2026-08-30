#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/orchestrator-selftest.XXXXXX")"
trap 'rm -rf "$TMP_ROOT"' EXIT

fail() {
  printf 'SELFTEST FAIL: %s\n' "$*" >&2
  exit 1
}

printf 'runtime bridge selftest\n'

# ---------------------------------------------------------------------------
# Agent Git: safe introspection passes through; repository mutation is blocked.
# ---------------------------------------------------------------------------
cat >"$TMP_ROOT/fake-git" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"${SELFTEST_GIT_TRACE:?}"
exit 0
EOF
chmod 700 "$TMP_ROOT/fake-git"
export SELFTEST_GIT_TRACE="$TMP_ROOT/git.trace"
export ORCHESTRATOR_REAL_GIT="$TMP_ROOT/fake-git"

bash "$ROOT/scripts/agent-git" status --short
if ! grep -Fxq 'status --short' "$SELFTEST_GIT_TRACE"; then
  fail 'read-only git status did not pass through'
fi

if bash "$ROOT/scripts/agent-git" push origin HEAD >/dev/null 2>&1; then
  fail 'agent git bridge allowed push'
fi
if bash "$ROOT/scripts/agent-git" commit -m nope >/dev/null 2>&1; then
  fail 'agent git bridge allowed commit'
fi

printf 'PASS agent-git read-only boundary\n'

# ---------------------------------------------------------------------------
# Parent Git bridge: every autonomous Git subprocess receives canonical identity.
# ---------------------------------------------------------------------------
cat >"$TMP_ROOT/fake-parent-git" <<'EOF'
#!/usr/bin/env bash
[[ "${GIT_AUTHOR_NAME:-}" == 'ZEKRITI Tarek' ]] || exit 61
[[ "${GIT_AUTHOR_EMAIL:-}" == '194770978+CHECKUPAUTO@users.noreply.github.com' ]] || exit 62
[[ "${GIT_COMMITTER_NAME:-}" == 'ZEKRITI Tarek' ]] || exit 63
[[ "${GIT_COMMITTER_EMAIL:-}" == '194770978+CHECKUPAUTO@users.noreply.github.com' ]] || exit 64
printf '%s\n' "$*" >>"${SELFTEST_PARENT_GIT_TRACE:?}"
exit 0
EOF
chmod 700 "$TMP_ROOT/fake-parent-git"
export SELFTEST_PARENT_GIT_TRACE="$TMP_ROOT/parent-git.trace"
export ORCHESTRATOR_REAL_GIT="$TMP_ROOT/fake-parent-git"
bash "$ROOT/scripts/git" status --short
if ! grep -Fxq 'status --short' "$SELFTEST_PARENT_GIT_TRACE"; then
  fail 'parent git bridge did not enforce canonical identity environment'
fi
printf 'PASS parent-git canonical identity\n'

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
printf '%s\n' "$*" >>"${SELFTEST_PUSH_TRACE:?}"
mode="${SELFTEST_PUSH_MODE:?}"
case "$mode:$*" in
  'permission:push origin HEAD')
    printf '%s\n' 'remote: Permission denied by repository policy' >&2
    printf '%s\n' '! [remote rejected] HEAD -> feature (permission denied)' >&2
    printf '%s\n' 'error: failed to push some refs to origin' >&2
    exit 1
    ;;
  'race:push origin HEAD')
    printf '%s\n' '! [rejected] HEAD -> feature (fetch first)' >&2
    printf '%s\n' 'error: failed to push some refs to origin' >&2
    exit 1
    ;;
  'race:branch --show-current')
    printf '%s\n' 'feature'
    ;;
  'race:rev-parse HEAD')
    printf '%s\n' 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
    ;;
  'race:update-ref refs/orchestrator/recovery/feature aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa') ;;
  'race:fetch origin feature') ;;
  'race:rev-parse --verify refs/remotes/origin/feature')
    printf '%s\n' 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
    ;;
  'race:merge-base --is-ancestor aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa refs/remotes/origin/feature') ;;
  'race:update-ref -d refs/orchestrator/recovery/feature') ;;
  *)
    printf 'unexpected fake push git command in %s mode: %s\n' "$mode" "$*" >&2
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
printf 'PASS parent-git permission rejection fails closed\n'

: >"$SELFTEST_PUSH_TRACE"
SELFTEST_PUSH_MODE=race bash "$ROOT/scripts/git" push origin HEAD >/dev/null 2>&1 ||   fail 'explicit fetch-first race did not enter safe recovery'
grep -Fxq 'branch --show-current' "$SELFTEST_PUSH_TRACE" ||   fail 'explicit fetch-first race did not inspect the current branch'
grep -Fxq 'fetch origin feature' "$SELFTEST_PUSH_TRACE" ||   fail 'explicit fetch-first race did not fetch the raced branch'
printf 'PASS parent-git explicit race recovery classification\n'

# Restore agent-boundary fake Git for any later checks.
export ORCHESTRATOR_REAL_GIT="$TMP_ROOT/fake-git"

# ---------------------------------------------------------------------------
# Agent GitHub CLI: views/checks pass through; mutations and raw API are blocked.
# ---------------------------------------------------------------------------
cat >"$TMP_ROOT/fake-gh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"${SELFTEST_GH_TRACE:?}"
exit 0
EOF
chmod 700 "$TMP_ROOT/fake-gh"
export SELFTEST_GH_TRACE="$TMP_ROOT/gh.trace"
export ORCHESTRATOR_REAL_GH="$TMP_ROOT/fake-gh"

bash "$ROOT/scripts/agent-gh" pr view 7 --repo Memorithm/ADA
if ! grep -Fxq 'pr view 7 --repo Memorithm/ADA' "$SELFTEST_GH_TRACE"; then
  fail 'read-only gh pr view did not pass through'
fi

if bash "$ROOT/scripts/agent-gh" pr merge 7 >/dev/null 2>&1; then
  fail 'agent gh bridge allowed PR merge'
fi
if bash "$ROOT/scripts/agent-gh" api repos/Memorithm/ADA >/dev/null 2>&1; then
  fail 'agent gh bridge allowed raw API escape hatch'
fi

printf 'PASS agent-gh read-only boundary\n'

# ---------------------------------------------------------------------------
# OpenCode environment: host HOME/XDG/plugin state must not leak into workers.
# ---------------------------------------------------------------------------
AGENT_HOME="$TMP_ROOT/agent-home"
AGENT_CONFIG="$AGENT_HOME/.config"
AGENT_DATA="$AGENT_HOME/.local/share"
AGENT_CACHE="$AGENT_HOME/.cache"
GH_CONFIG="$TMP_ROOT/real-gh-config"
mkdir -p "$GH_CONFIG"

cat >"$TMP_ROOT/fake-opencode-core" <<'EOF'
#!/usr/bin/env bash
[[ "$HOME" == "${SELFTEST_EXPECT_HOME:?}" ]] || exit 41
[[ "$XDG_CONFIG_HOME" == "${SELFTEST_EXPECT_CONFIG:?}" ]] || exit 42
[[ "$XDG_DATA_HOME" == "${SELFTEST_EXPECT_DATA:?}" ]] || exit 43
[[ "$XDG_CACHE_HOME" == "${SELFTEST_EXPECT_CACHE:?}" ]] || exit 44
[[ "$GH_CONFIG_DIR" == "${SELFTEST_EXPECT_GH:?}" ]] || exit 45
[[ "$OPENCODE_DISABLE_PROJECT_CONFIG" == "1" ]] || exit 46
[[ "$OPENCODE_DISABLE_EXTERNAL_SKILLS" == "1" ]] || exit 47
[[ "$OPENCODE_DISABLE_DEFAULT_PLUGINS" == "1" ]] || exit 48
[[ "$OPENCODE_DISABLE_CLAUDE_CODE" == "1" ]] || exit 49
[[ "$OPENCODE_DISABLE_LSP_DOWNLOAD" == "1" ]] || exit 50
[[ "$OPENCODE_DISABLE_MODELS_FETCH" == "1" ]] || exit 51
[[ "$OPENCODE_AUTO_SHARE" == "0" ]] || exit 52
[[ "$OPENCODE_PURE" == "1" ]] || exit 53
printf 'isolated-ok\n'
EOF
chmod 700 "$TMP_ROOT/fake-opencode-core"

export ORCHESTRATOR_OPENCODE_CORE="$TMP_ROOT/fake-opencode-core"
export ORCHESTRATOR_AGENT_HOME="$AGENT_HOME"
export ORCHESTRATOR_AGENT_CONFIG_DIR="$AGENT_CONFIG"
export ORCHESTRATOR_AGENT_DATA_DIR="$AGENT_DATA"
export ORCHESTRATOR_AGENT_CACHE_DIR="$AGENT_CACHE"
export ORCHESTRATOR_GH_CONFIG_DIR="$GH_CONFIG"
export SELFTEST_EXPECT_HOME="$AGENT_HOME"
export SELFTEST_EXPECT_CONFIG="$AGENT_CONFIG"
export SELFTEST_EXPECT_DATA="$AGENT_DATA"
export SELFTEST_EXPECT_CACHE="$AGENT_CACHE"
export SELFTEST_EXPECT_GH="$GH_CONFIG"

isolation_output="$(bash "$ROOT/scripts/opencode-env" run sentinel)"
[[ "$isolation_output" == 'isolated-ok' ]] || fail 'OpenCode isolation wrapper did not execute managed core'

printf 'PASS OpenCode HOME/XDG/plugin isolation\n'

# ---------------------------------------------------------------------------
# Cargo bridge: unstable rustfmt config must select the unique CI nightly even
# when a stable/MSRV fmt invocation is also present elsewhere in workflows.
# ---------------------------------------------------------------------------
mkdir -p "$TMP_ROOT/repo/.github/workflows"
cat >"$TMP_ROOT/repo/rustfmt.toml" <<'EOF'
unstable_features = true
brace_style = "PreferSameLine"
EOF
cat >"$TMP_ROOT/repo/.github/workflows/ci.yml" <<'EOF'
name: ci
jobs:
  fmt:
    steps:
      - run: cargo +nightly-2026-07-02 fmt --all -- --check
  msrv:
    steps:
      - run: cargo +1.89.0 fmt --all -- --check
EOF
cat >"$TMP_ROOT/fake-cargo" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"${SELFTEST_CARGO_TRACE:?}"
exit 0
EOF
chmod 700 "$TMP_ROOT/fake-cargo"
export ORCHESTRATOR_REAL_CARGO="$TMP_ROOT/fake-cargo"
export SELFTEST_CARGO_TRACE="$TMP_ROOT/cargo.trace"

(
  cd "$TMP_ROOT/repo"
  bash "$ROOT/scripts/cargo" fmt --all -- --check
)
if ! grep -Fxq '+nightly-2026-07-02 fmt --all -- --check' "$SELFTEST_CARGO_TRACE"; then
  fail 'cargo bridge did not select the unique nightly for unstable rustfmt config'
fi

printf 'PASS cargo CI-pinned rustfmt selection\n'
printf 'runtime bridge selftest: PASS\n'
