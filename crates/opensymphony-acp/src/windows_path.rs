//! Reject reparse points before Windows callback APIs can follow them.
use std::{io, os::windows::fs::MetadataExt, path::Path};

pub(super) async fn validate_components(
    root: &Path,
    relative: &Path,
    missing: bool,
) -> io::Result<()> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match tokio::fs::symlink_metadata(&current).await {
            Ok(metadata) => {
                // Includes junctions and other reparse tags that is_symlink misses.
                const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
                if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    return Err(io::ErrorKind::InvalidInput.into());
                }
            }
            Err(error) if missing && error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn junctions_cannot_supply_callback_files_or_terminal_directories() {
        let root = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        std::fs::write(outside.path().join("existing"), "outside").expect("file");
        let junction = root.path().join("junction");
        assert!(
            std::process::Command::new("cmd.exe")
                .args(["/C", "mklink", "/J"])
                .arg(&junction)
                .arg(outside.path())
                .status()
                .expect("create junction")
                .success()
        );
        // Existing reads, nonexistent writes and terminal cwd all pass through
        // the same component validation used by Services::path.
        for (path, missing) in [
            ("junction/existing", false),
            ("junction/new/destination", true),
            ("junction", false),
        ] {
            assert_eq!(
                validate_components(root.path(), Path::new(path), missing)
                    .await
                    .expect_err("junction rejected")
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
        std::fs::create_dir(root.path().join("regular")).expect("directory");
        validate_components(root.path(), Path::new("regular/new"), true)
            .await
            .expect("ordinary missing write allowed");
        validate_components(root.path(), Path::new("regular"), false)
            .await
            .expect("ordinary cwd allowed");
        std::fs::remove_dir(junction).expect("remove junction only");
        assert!(outside.path().join("existing").is_file());
    }
}
