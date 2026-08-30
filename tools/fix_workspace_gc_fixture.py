from pathlib import Path

path = Path("src/workspace_gc.rs")
text = path.read_text()
old = '''        fs::write(dirty.join("tracked.txt"), b"tracked\\n").unwrap();
        git(&dirty, &["add", "tracked.txt"]);
        git(&dirty, &["commit", "-m", "local-only"]);'''
new = '''        git(&dirty, &["checkout", "--", "tracked.txt"]);
        fs::write(dirty.join("local-only.txt"), b"local commit\\n").unwrap();
        git(&dirty, &["add", "local-only.txt"]);
        git(&dirty, &["commit", "-m", "local-only"]);'''
if text.count(old) != 1:
    raise SystemExit("workspace GC local-only fixture anchor changed")
path.write_text(text.replace(old, new, 1))
