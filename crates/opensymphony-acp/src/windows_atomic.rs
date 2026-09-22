//! Handle-only Windows stage promotion and cancellation cleanup.
use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    io,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
};
use windows_sys::{
    Wdk::Storage::FileSystem::{
        FILE_RENAME_INFORMATION, FileRenameInformation, NtSetInformationFile,
    },
    Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE, RtlNtStatusToDosError},
        Storage::FileSystem::{
            DELETE, FILE_DISPOSITION_INFO, FILE_SHARE_READ, FileDispositionInfo,
            SetFileInformationByHandle,
        },
        System::IO::IO_STATUS_BLOCK,
    },
};

pub(super) fn create_stage(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
        // Neither another writer nor a rename can change this stage before
        // its handle-relative promotion. Clones refer to the same file object.
        .share_mode(FILE_SHARE_READ)
        .create_new(true)
        .open(path)
}

#[repr(C)]
struct RenameInformation {
    replace: u32,
    root: HANDLE,
    name_bytes: u32,
    name: [u16; 256],
}
const _: () = {
    assert!(
        std::mem::offset_of!(RenameInformation, root)
            == std::mem::offset_of!(FILE_RENAME_INFORMATION, RootDirectory)
    );
    assert!(
        std::mem::offset_of!(RenameInformation, name_bytes)
            == std::mem::offset_of!(FILE_RENAME_INFORMATION, FileNameLength)
    );
    assert!(
        std::mem::offset_of!(RenameInformation, name)
            == std::mem::offset_of!(FILE_RENAME_INFORMATION, FileName)
    );
    assert!(
        std::mem::align_of::<RenameInformation>()
            == std::mem::align_of::<FILE_RENAME_INFORMATION>()
    );
};

#[allow(unsafe_code)]
pub(super) fn replace_in_same_directory(file: &File, leaf: &OsStr) -> io::Result<()> {
    let name: Vec<_> = leaf.encode_wide().collect();
    if name.is_empty()
        || name.len() > 255
        || name.iter().any(|c| matches!(*c, 0 | 47 | 58 | 92))
        || leaf == "."
        || leaf == ".."
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut information = RenameInformation {
        replace: 1,
        root: std::ptr::null_mut(),
        name_bytes: (name.len() * 2) as u32,
        name: [0; 256],
    };
    information.name[..name.len()].copy_from_slice(&name);
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: file is a live, synchronous DELETE-access handle. The aligned
    // repr(C) buffer matches the SDK layout (asserted above), includes the full
    // bounded UTF-16 leaf, and lives through this synchronous call. A null root
    // plus a simple leaf renames within the source file's directory; source and
    // all ancestor handles deny delete/write sharing, so neither can move.
    // Win32 path rename opens the target parent for write and conflicts with
    // those guards. The NT same-directory form requires no such parent open.
    let result = unsafe {
        NtSetInformationFile(
            file.as_raw_handle(),
            &mut status,
            std::ptr::from_ref(&information).cast(),
            std::mem::size_of_val(&information) as u32,
            FileRenameInformation,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        // SAFETY: pure status-code translation with no pointers or ownership.
        Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(result) } as i32,
        ))
    }
}

#[allow(unsafe_code)]
pub(super) fn delete_on_close(file: &File) -> io::Result<()> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: file is a live DELETE-access handle and disposition is a correctly
    // sized SDK structure kept alive throughout this synchronous call. This
    // marks the owned file object, never a potentially substituted path.
    let result = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            std::ptr::from_ref(&disposition).cast(),
            std::mem::size_of_val(&disposition) as u32,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
