#!/usr/bin/env python3
from pathlib import Path

main_path = Path("src/main.rs")
policy_path = Path("src/merge_policy.rs")
main = main_path.read_text()
policy = policy_path.read_text()

old = '''    if final_metadata.head_sha != metadata.head_sha
        || final_metadata.author != metadata.author
        || final_metadata.base_branch != metadata.base_branch
        || final_metadata.cross_repository != metadata.cross_repository
'''
new = '''    if final_metadata.head_sha != metadata.head_sha
        || final_metadata.head_branch != metadata.head_branch
        || final_metadata.author != metadata.author
        || final_metadata.base_branch != metadata.base_branch
        || final_metadata.cross_repository != metadata.cross_repository
'''
if main.count(old) != 1:
    raise SystemExit(f"final merge metadata guard: expected exactly one match, found {main.count(old)}")
main = main.replace(old, new, 1)

old_import = "use std::path::{Path, PathBuf};\n"
if policy.count(old_import) != 1:
    raise SystemExit("merge policy import shape changed")
policy = policy.replace(old_import, "use std::path::PathBuf;\n", 1)

main_path.write_text(main)
policy_path.write_text(policy)
