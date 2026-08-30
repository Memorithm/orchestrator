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
text = text.replace(old, new, 1)

old = '''        if let Err(error) = fs::remove_file(&marker) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(format!(
                    "removed workspace {} but failed to remove usage marker {}: {error}",
                    workspace.display(),
                    marker.display()
                ));
            }
        }'''
new = '''        if let Err(error) = fs::remove_file(&marker)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(format!(
                "removed workspace {} but failed to remove usage marker {}: {error}",
                workspace.display(),
                marker.display()
            ));
        }'''
if text.count(old) != 1:
    raise SystemExit("workspace GC marker cleanup anchor changed")
text = text.replace(old, new, 1)

path.write_text(text)
