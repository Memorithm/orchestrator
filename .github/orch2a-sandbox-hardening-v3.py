from pathlib import Path


def replace_once(data: str, old: str, new: str, label: str) -> str:
    count = data.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return data.replace(old, new, 1)


sandbox_path = Path("scripts/validation-sandbox")
sandbox = sandbox_path.read_text()
sandbox = replace_once(
    sandbox,
    'SOURCE_ROOT="${ORCHESTRATOR_SOURCE_ROOT:-}"\n',
    "",
    "remove source root variable",
)
sandbox = replace_once(
    sandbox,
    '''if [[ -n "$SOURCE_ROOT" && -d "$SOURCE_ROOT" ]]; then
  SOURCE_ROOT="$(realpath -m "$SOURCE_ROOT")"
  if [[ "$SOURCE_ROOT" != "$WORKSPACE" ]]; then
    add_tmpfs_mask "$SOURCE_ROOT"
  fi
fi
''',
    "",
    "remove source root mask",
)
sandbox_path.write_text(sandbox)

start_path = Path("scripts/start.sh")
start = start_path.read_text()
start = replace_once(
    start,
    'export ORCHESTRATOR_SOURCE_ROOT="$ROOT"\n',
    "",
    "remove source root export",
)
start_path.write_text(start)

ci_path = Path(".github/workflows/ci.yml")
ci = ci_path.read_text()
ci = replace_once(
    ci,
    "      - uses: actions/checkout@v4\n",
    "      - uses: actions/checkout@v4\n        with:\n          persist-credentials: false\n",
    "disable persisted checkout credentials",
)
ci = replace_once(
    ci,
    '            HOME="$HOME" \\\n',
    '            HOME="/root" \\\n',
    "root deployment home",
)
ci = replace_once(
    ci,
    '            ORCHESTRATOR_SOURCE_ROOT="$GITHUB_WORKSPACE" \\\n',
    "",
    "remove source root CI environment",
)
ci_path.write_text(ci)
