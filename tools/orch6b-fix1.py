from pathlib import Path

path = Path("src/hardware_dispatch.rs")
text = path.read_text()
text = text.replace("use std::path::{Path, PathBuf};\n", "use std::path::Path;\n", 1)
text = text.replace(
    "    use std::fs;\n    use std::time::{SystemTime, UNIX_EPOCH};\n",
    "    use std::fs;\n    use std::path::PathBuf;\n    use std::time::{SystemTime, UNIX_EPOCH};\n",
    1,
)
start = text.index("fn ensure_managed_directory(")
end = text.index("fn read_regular_bounded(", start)
replacement = r'''fn ensure_managed_directory(
    data_root: &Path,
    state_root: &Path,
    directory: &Path,
) -> Result<(), String> {
    let canonical_data = fs::canonicalize(data_root).map_err(|error| {
        format!(
            "failed to canonicalize orchestrator data root {}: {error}",
            data_root.display()
        )
    })?;
    if !state_root.starts_with(data_root) || !directory.starts_with(state_root) {
        return Err(format!(
            "hardware dispatch state path is outside orchestrator data root: {}",
            directory.display()
        ));
    }

    let relative = directory.strip_prefix(data_root).map_err(|_| {
        format!(
            "hardware dispatch directory is outside orchestrator data root: {}",
            directory.display()
        )
    })?;
    let mut current = data_root.to_path_buf();
    for component in relative.components() {
        use std::path::Component;
        let Component::Normal(component) = component else {
            return Err("hardware dispatch directory contains a non-normal component".to_owned());
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "hardware dispatch directory component must be a non-symlink directory: {}",
                        current.display()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| {
                    format!(
                        "failed to create hardware dispatch directory component {}: {error}",
                        current.display()
                    )
                })?;
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect hardware dispatch directory component {}: {error}",
                    current.display()
                ));
            }
        }
        let canonical_current = fs::canonicalize(&current).map_err(|error| {
            format!(
                "failed to canonicalize hardware dispatch directory component {}: {error}",
                current.display()
            )
        })?;
        if !canonical_current.starts_with(&canonical_data) {
            return Err(format!(
                "hardware dispatch directory component escapes orchestrator data root: {}",
                current.display()
            ));
        }
    }

    let state_metadata = fs::symlink_metadata(state_root).map_err(|error| {
        format!(
            "failed to inspect hardware dispatch state root {}: {error}",
            state_root.display()
        )
    })?;
    if state_metadata.file_type().is_symlink() || !state_metadata.is_dir() {
        return Err(format!(
            "hardware dispatch state root must be a non-symlink directory: {}",
            state_root.display()
        ));
    }
    Ok(())
}

'''
text = text[:start] + replacement + text[end:]
path.write_text(text)
print("ORCH6b managed directory fix applied")
