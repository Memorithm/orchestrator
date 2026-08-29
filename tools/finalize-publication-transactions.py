#!/usr/bin/env python3
from pathlib import Path

path = Path("src/main.rs")
source = path.read_text()
old = r'''            r#"if length == 0 then "" else "#\(.[0].number) \(.[0].state) \(.[0].url)" end"#,
'''
new = r'''            r##"if length == 0 then "" else "#\(.[0].number) \(.[0].state) \(.[0].url)" end"##,
'''
count = source.count(old)
if count != 1:
    raise SystemExit(f"publication jq raw string: expected exactly one match, found {count}")
path.write_text(source.replace(old, new, 1))
