use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const CONFIG_FILENAMES: [&str; 2] = [".shuck.toml", "shuck.toml"];

pub(crate) fn resolve_project_root_for_input(input: &Path) -> io::Result<PathBuf> {
    let base_dir = base_dir_for_input(input)?;
    Ok(find_config_root(&base_dir)?.unwrap_or(base_dir))
}

pub(crate) fn resolve_project_root_for_file(
    file: &Path,
    fallback_start: &Path,
) -> io::Result<PathBuf> {
    let start = file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| fallback_start.to_path_buf());
    Ok(find_config_root(&start)?.unwrap_or_else(|| normalize_path(fallback_start)))
}

fn base_dir_for_input(input: &Path) -> io::Result<PathBuf> {
    let normalized = normalize_path(input);
    let metadata = fs::metadata(&normalized)?;
    if metadata.is_dir() {
        Ok(normalized)
    } else {
        Ok(normalized
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")))
    }
}

fn find_config_root(start: &Path) -> io::Result<Option<PathBuf>> {
    let start = normalize_path(start);

    let mut current = start.as_path();
    loop {
        for filename in CONFIG_FILENAMES {
            let candidate = current.join(filename);
            match fs::metadata(&candidate) {
                Ok(metadata) if metadata.is_file() => return Ok(Some(current.to_path_buf())),
                Ok(_) => continue,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            }
        }

        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent;
    }

    Ok(None)
}

fn normalize_path(path: &Path) -> PathBuf {
    path.components().collect()
}
