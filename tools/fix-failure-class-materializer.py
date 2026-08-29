#!/usr/bin/env python3
from pathlib import Path
import re

path = Path("src/main.rs")
source = path.read_text()
old = '''            failure_max_cooldown_secs: 60,
            quarantine_after_failures: 4,
'''
new = '''            failure_max_cooldown_secs: 60,
            transient_failure_cooldown_secs: 1,
            quarantine_after_failures: 4,
'''
count = source.count(old)
if count != 1:
    raise SystemExit(f"selection-test RetryPolicy literal: expected exactly one match, found {count}")
source = source.replace(old, new, 1)

# Fail closed if any explicit RetryPolicy literal still omits the transient field.
for index, match in enumerate(re.finditer(r"state::RetryPolicy\s*\{", source), start=1):
    tail = source[match.end():]
    end = tail.find("\n        };")
    if end < 0:
        end = tail.find("\n            },")
    if end < 0:
        raise SystemExit(f"RetryPolicy literal {index}: unable to find deterministic end")
    body = tail[:end]
    if "transient_failure_cooldown_secs" not in body:
        raise SystemExit(f"RetryPolicy literal {index}: transient failure cooldown missing")

path.write_text(source)
