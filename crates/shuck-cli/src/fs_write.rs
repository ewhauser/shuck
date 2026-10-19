use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Replaces the contents of `path` without ever leaving a truncated file behind.
///
/// The new contents are written to a temporary file in the same directory and
/// renamed over the original, so readers see either the old or the new file.
/// The original permissions are carried over. When `path` is a symlink, the
/// link's target is rewritten and the link itself is left in place.
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let target = fs::canonicalize(path)?;
    let permissions = fs::metadata(&target)?.permissions();
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::other("file has no parent directory"))?;

    let mut temp_file = tempfile::Builder::new()
        .prefix(".shuck-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temp_file.write_all(contents)?;
    temp_file.flush()?;
    fs::set_permissions(temp_file.path(), permissions)?;
    temp_file.persist(&target).map_err(|err| err.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_contents_and_keeps_permissions() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("script.sh");
        fs::write(&path, "old\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
        }

        write_atomic(&path, b"new\n").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o751);
        }
        let leftovers = fs::read_dir(tempdir.path()).unwrap().count();
        assert_eq!(leftovers, 1);
    }

    #[cfg(unix)]
    #[test]
    fn rewrites_symlink_target_and_keeps_the_link() {
        let tempdir = tempfile::tempdir().unwrap();
        let target = tempdir.path().join("target.sh");
        let link = tempdir.path().join("link.sh");
        fs::write(&target, "old\n").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        write_atomic(&link, b"new\n").unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "new\n");
    }
}
