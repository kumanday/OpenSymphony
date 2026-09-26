//! Pin Windows callback paths so validation cannot be invalidated by a junction swap.
use std::{
    io,
    os::windows::fs::MetadataExt,
    path::{Component, Path},
};
use tokio::fs::{File, OpenOptions};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_SHARE_READ: u32 = 1;

/// Keep every ancestor pinned from the drive/share root. Excluding write and
/// delete sharing prevents renaming a checked directory or turning it into a
/// reparse point until the callback finishes using the path.
pub(super) async fn pin_directory(path: &Path, create: bool) -> io::Result<Vec<File>> {
    if !path.is_absolute() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut current = std::path::PathBuf::new();
    let mut guards = Vec::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        current.push(component);
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        if create && matches!(component, Component::Normal(_)) {
            match tokio::fs::create_dir(&current).await {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        let directory = OpenOptions::new()
            // Attribute-only handles do not participate in Windows sharing checks.
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(&current)
            .await?;
        let metadata = directory.metadata().await?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        guards.push(directory);
    }
    Ok(guards)
}

pub(super) async fn open_file(path: &Path, write: bool) -> io::Result<(File, Vec<File>)> {
    let guards = pin_directory(path.parent().ok_or(io::ErrorKind::InvalidInput)?, write).await?;
    // OPEN_ALWAYS (create without truncate) and OPEN_REPARSE_POINT let us
    // inspect the opened leaf before any content mutation occurs.
    let file = OpenOptions::new()
        .read(!write)
        .write(write)
        .create(write)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .await?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok((file, guards))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    fn junction(link: &Path, target: &Path) {
        assert!(
            std::process::Command::new("cmd.exe")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .status()
                .expect("create junction")
                .success()
        );
    }

    #[tokio::test]
    async fn junctions_cannot_supply_callback_files_or_terminal_directories() {
        let root = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        std::fs::write(outside.path().join("existing"), "outside").expect("file");
        let link = root.path().join("junction");
        junction(&link, outside.path());
        assert!(open_file(&link.join("existing"), false).await.is_err());
        assert!(
            open_file(&link.join("new/destination"), true)
                .await
                .is_err()
        );
        assert!(pin_directory(&link, false).await.is_err());
        assert!(!outside.path().join("new").exists());
        std::fs::remove_dir(link).expect("remove junction only");
        assert_eq!(
            std::fs::read_to_string(outside.path().join("existing")).expect("untouched"),
            "outside"
        );
    }

    #[tokio::test]
    async fn pinned_callback_paths_reject_swaps_until_file_io_and_spawn_finish() {
        let root = tempfile::tempdir().expect("workspace");
        let inside = root.path().join("inside");
        std::fs::create_dir(&inside).expect("directory");
        let path = inside.join("new/leaf");
        let (mut file, guards) = open_file(&path, true).await.expect("contained write");
        let moved = root.path().join("moved");
        assert!(
            std::fs::rename(&inside, &moved).is_err(),
            "ancestor must stay pinned"
        );
        assert!(
            std::fs::rename(&path, inside.join("other")).is_err(),
            "leaf must stay pinned"
        );
        file.write_all(b"inside").await.expect("write while pinned");
        file.flush().await.expect("flush");
        drop(file);
        drop(guards);
        let cwd = pin_directory(&inside, false).await.expect("terminal cwd");
        assert!(std::fs::rename(&inside, &moved).is_err());
        assert!(
            tokio::process::Command::new("cmd.exe")
                .args(["/C", "exit", "0"])
                .current_dir(&inside)
                .status()
                .await
                .expect("spawn under pinned cwd")
                .success()
        );
        drop(cwd);
        std::fs::rename(&inside, &moved).expect("unpin releases rename exclusion");
        assert_eq!(
            std::fs::read(moved.join("new/leaf")).expect("contained output"),
            b"inside"
        );
    }
}
