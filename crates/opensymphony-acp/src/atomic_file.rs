//! Cancel-safe callback writes: only a completed stage may replace the destination.
use std::{io, path::Path};
use tokio::fs::File;

pub(super) struct AtomicFile {
    pub file: File,
    #[cfg(unix)]
    directory: std::os::fd::OwnedFd,
    #[cfg(unix)]
    temporary: std::ffi::OsString,
    #[cfg(unix)]
    destination: std::ffi::OsString,
    #[cfg(not(unix))]
    temporary: Option<tempfile::NamedTempFile>,
    #[cfg(not(unix))]
    destination: std::path::PathBuf,
    #[cfg(windows)]
    _guards: Vec<File>,
}
impl AtomicFile {
    pub async fn new(root: &Path, path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use rustix::fs::{FileType, Mode, OFlags, fchmod, fstat, mkdirat, open, openat};
            let relative = path
                .strip_prefix(root)
                .map_err(|_| io::ErrorKind::InvalidInput)?;
            let destination = relative
                .file_name()
                .ok_or(io::ErrorKind::InvalidInput)?
                .to_owned();
            let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            let mut directory = open(root, flags, Mode::empty())?;
            for component in relative
                .parent()
                .ok_or(io::ErrorKind::InvalidInput)?
                .components()
            {
                if !matches!(component, std::path::Component::Normal(_)) {
                    return Err(io::ErrorKind::InvalidInput.into());
                }
                match mkdirat(
                    &directory,
                    component.as_os_str(),
                    Mode::from_raw_mode(0o700),
                ) {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                    Err(error) => return Err(error.into()),
                }
                directory = openat(&directory, component.as_os_str(), flags, Mode::empty())?;
            }
            // Preserve the original file's write authorization and permissions,
            // without truncating it or following a substituted leaf symlink.
            let permissions = match openat(
                &directory,
                &destination,
                OFlags::WRONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(file) => {
                    let metadata = fstat(&file)?;
                    if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile {
                        return Err(io::ErrorKind::InvalidInput.into());
                    }
                    Some(Mode::from_raw_mode(metadata.st_mode))
                }
                Err(rustix::io::Errno::NOENT) => None,
                Err(error) => return Err(error.into()),
            };
            let temporary =
                std::ffi::OsString::from(format!(".opensymphony-write-{}", uuid::Uuid::new_v4()));
            let file = openat(
                &directory,
                &temporary,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )?;
            let result = Self {
                file: File::from_std(std::fs::File::from(file)),
                directory,
                temporary,
                destination,
            };
            if let Some(permissions) = permissions {
                fchmod(&result.file, permissions)?;
            }
            Ok(result)
        }
        #[cfg(not(unix))]
        {
            let _ = root;
            let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
            #[cfg(windows)]
            let guards = super::windows_path::pin_directory(parent, true).await?;
            #[cfg(not(windows))]
            tokio::fs::create_dir_all(parent).await?;
            let permissions = match tokio::fs::symlink_metadata(path).await {
                Ok(metadata) => {
                    #[cfg(windows)]
                    {
                        use std::os::windows::fs::MetadataExt;
                        if metadata.file_attributes() & 0x400 != 0 {
                            return Err(io::ErrorKind::InvalidInput.into());
                        }
                    }
                    if !metadata.is_file() || metadata.permissions().readonly() {
                        return Err(io::ErrorKind::InvalidInput.into());
                    }
                    Some(metadata.permissions())
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            let temporary = tempfile::Builder::new()
                .prefix(".opensymphony-write-")
                .tempfile_in(parent)?;
            if let Some(permissions) = permissions {
                temporary.as_file().set_permissions(permissions)?;
            }
            let file = File::from_std(temporary.as_file().try_clone()?);
            Ok(Self {
                file,
                temporary: Some(temporary),
                destination: path.into(),
                #[cfg(windows)]
                _guards: guards,
            })
        }
    }

    // No await between the caller's last cancellation check and atomic replace.
    pub fn commit(mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            rustix::fs::renameat(
                &self.directory,
                &self.temporary,
                &self.directory,
                &self.destination,
            )?;
            self.temporary.clear();
        }
        #[cfg(not(unix))]
        {
            // Close staging data handles before Windows promotion. Parent
            // guards remain owned by self until the atomic rename finishes.
            drop(self.file);
            self.temporary
                .take()
                .ok_or(io::ErrorKind::InvalidInput)?
                .into_temp_path()
                .persist(&self.destination)
                .map_err(|error| error.error)?;
        }
        Ok(())
    }
}
#[cfg(unix)]
impl Drop for AtomicFile {
    fn drop(&mut self) {
        if !self.temporary.is_empty() {
            let _ = rustix::fs::unlinkat(
                &self.directory,
                &self.temporary,
                rustix::fs::AtFlags::empty(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn partial_stage_drop_preserves_original_and_completed_stage_replaces_it() {
        let root = tempfile::tempdir().expect("root");
        let root = root.path().canonicalize().expect("canonical");
        let path = root.join("file");
        std::fs::write(&path, "original").expect("original");
        let mut stage = AtomicFile::new(&root, &path).await.expect("stage");
        stage
            .file
            .write_all(b"partial")
            .await
            .expect("partial write");
        stage.file.flush().await.expect("flush partial");
        assert_eq!(
            std::fs::read(&path).expect("original survives staging"),
            b"original"
        );
        drop(stage);
        assert_eq!(std::fs::read_dir(&root).expect("entries").count(), 1);
        assert_eq!(
            std::fs::read(&path).expect("cancel leaves original"),
            b"original"
        );
        let mut stage = AtomicFile::new(&root, &path).await.expect("stage");
        stage.file.write_all(b"complete").await.expect("write");
        stage.file.flush().await.expect("flush");
        stage.commit().expect("commit");
        assert_eq!(std::fs::read(&path).expect("replaced"), b"complete");
        assert_eq!(std::fs::read_dir(&root).expect("entries").count(), 1);
    }

    #[tokio::test]
    async fn stage_io_error_preserves_original_and_removes_temporary_file() {
        let root = tempfile::tempdir().expect("root");
        let root = root.path().canonicalize().expect("canonical");
        let path = root.join("file");
        std::fs::write(&path, "original").expect("original");
        let mut stage = AtomicFile::new(&root, &path).await.expect("stage");
        #[cfg(unix)]
        let temporary_path = root.join(&stage.temporary);
        #[cfg(not(unix))]
        let temporary_path = stage.temporary.as_ref().expect("stage").path().to_owned();
        // Reopen the actual stage read-only to deterministically inject a write
        // failure at the same I/O boundary used by the callback.
        stage.file = File::from_std(std::fs::File::open(&temporary_path).expect("read-only stage"));
        assert!(
            stage.file.write_all(b"replacement").await.is_err()
                || stage.file.flush().await.is_err()
        );
        drop(stage);
        assert_eq!(std::fs::read(&path).expect("unchanged"), b"original");
        assert_eq!(std::fs::read_dir(&root).expect("entries").count(), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn atomic_write_keeps_permissions_and_descriptor_relative_parent() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().expect("root");
        let root = root.path().canonicalize().expect("canonical");
        let inside = root.join("inside");
        std::fs::create_dir(&inside).expect("inside");
        let path = inside.join("file");
        std::fs::write(&path, "original").expect("original");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).expect("mode");
        let mut stage = AtomicFile::new(&root, &path).await.expect("stage");
        let outside = tempfile::tempdir().expect("outside");
        std::fs::write(outside.path().join("file"), "outside").expect("outside");
        std::fs::rename(&inside, root.join("moved")).expect("rename");
        std::os::unix::fs::symlink(outside.path(), &inside).expect("swap");
        stage.file.write_all(b"complete").await.expect("write");
        stage.file.flush().await.expect("flush");
        stage.commit().expect("commit into held parent");
        assert_eq!(
            std::fs::read(root.join("moved/file")).expect("file"),
            b"complete"
        );
        assert_eq!(
            std::fs::metadata(root.join("moved/file"))
                .expect("mode")
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        assert_eq!(
            std::fs::read(outside.path().join("file")).expect("outside"),
            b"outside"
        );
        assert!(AtomicFile::new(&root, &path).await.is_err());
    }
}
