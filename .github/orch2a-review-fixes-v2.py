from pathlib import Path

path = Path("src/main.rs")
data = path.read_text()
old = '''        fs::write(workspace.join("src/lib.rs"), "pub fn smoke() {}\\n").unwrap();
        let config = orch2_validation_test_config(&data_root);
'''
new = '''        fs::write(workspace.join("src/lib.rs"), "pub fn smoke() {}\\n").unwrap();
        run_in_dir(&workspace, "cargo", &["generate-lockfile", "--offline"]).unwrap();
        let config = orch2_validation_test_config(&data_root);
'''
if data.count(old) != 1:
    raise SystemExit(f"cargo fixture lockfile anchor count={data.count(old)}")
path.write_text(data.replace(old, new, 1))
