/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `sys/mount.h`, file system statistics

use crate::dyld::{export_c_func, FunctionExports};
use crate::fs::{resolve_path, Fs, GuestPath};
use crate::libc::dirent::MAXPATHLEN;
use crate::libc::errno::{set_errno, EBADF, EFAULT, EINVAL, ENOENT, ENOTDIR};
use crate::libc::posix_io::stat::uid_t;
use crate::libc::posix_io::{FileDescriptor, STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO};
use crate::mem::{ConstPtr, MutPtr, SafeRead};
use crate::Environment;

const MFSTYPENAMELEN: usize = 16;
// Darwin mount flags, not the differently numbered POSIX statvfs flags.
const MNT_RDONLY: u32 = 0x1;
const MNT_NOSUID: u32 = 0x8;
const MNT_NODEV: u32 = 0x10;
const MNT_LOCAL: u32 = 0x1000;

#[allow(non_camel_case_types)]
#[derive(Default, Debug, Copy, Clone)]
#[repr(C, packed)]
pub struct fsid_t {
    pub val: [i32; 2],
}

#[allow(non_camel_case_types)]
#[derive(Debug)]
#[repr(C, packed)]
pub struct statfs {
    pub f_bsize: u32,
    pub f_iosize: i32,
    pub f_blocks: u64,
    pub f_bfree: u64,
    pub f_bavail: u64,
    pub f_files: u64,
    pub f_ffree: u64,
    pub f_fsid: fsid_t,
    pub f_owner: uid_t,
    pub f_type: u32,
    pub f_flags: u32,
    pub f_fssubtype: u32,
    pub f_fstypename: [u8; MFSTYPENAMELEN],
    pub f_mntonname: [u8; MAXPATHLEN],
    pub f_mntfromname: [u8; MAXPATHLEN],
    pub f_reserved: [u32; 8],
}
unsafe impl SafeRead for statfs {}

fn fake_statfs() -> statfs {
    // Values are taken from a test run of iOS 4.3 Simulator
    let mut statfs = statfs {
        f_bsize: 4096,
        f_iosize: 1048576,
        f_blocks: 16567314,
        f_bfree: 12461147,
        f_bavail: 12397147,
        f_files: 16567312,
        f_ffree: 12397147,
        f_fsid: fsid_t {
            val: [234881026, 17],
        },
        f_owner: 0,
        f_type: 17,
        f_flags: MNT_NOSUID | MNT_NODEV | MNT_LOCAL,
        f_fssubtype: 1,
        f_fstypename: [b'\0'; MFSTYPENAMELEN],
        f_mntonname: [b'\0'; MAXPATHLEN],
        f_mntfromname: [b'\0'; MAXPATHLEN],
        f_reserved: [0u32; 8],
    };
    statfs.f_fstypename[..3].copy_from_slice(b"hfs");
    statfs.f_mntonname[..1].copy_from_slice(b"/");
    statfs.f_mntfromname[..12].copy_from_slice(b"/dev/disk0s2");
    statfs
}

/// Internal helper for `statfs`, not a part of the API.
pub fn statfs_inner(env: &mut Environment, path: ConstPtr<u8>) -> Result<statfs, i32> {
    if path.is_null() {
        return Err(EFAULT);
    }
    let path = env.mem.cstr_at_utf8(path).map_err(|_| EINVAL)?;
    statfs_for_path(&env.fs, path)
}

fn statfs_for_path(fs: &Fs, path: &str) -> Result<statfs, i32> {
    if path.is_empty() {
        return Err(ENOENT);
    }
    let guest = GuestPath::new(path);
    let (exists, _, writable, _) = fs.access(guest);
    if !exists {
        // Distinguish a missing entry from traversal through a regular file.
        let components = resolve_path(guest, Some(fs.working_directory()));
        let mut prefix = String::new();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            prefix.push('/');
            prefix.push_str(component);
            if fs.is_file(GuestPath::new(&prefix)) {
                return Err(ENOTDIR);
            }
            if !fs.exists(GuestPath::new(&prefix)) {
                return Err(ENOENT);
            }
        }
        return Err(ENOENT);
    }
    Ok(statfs_for_access(writable))
}

fn statfs_for_access(writable: bool) -> statfs {
    // Capacity remains the existing emulated filesystem's simulator-derived
    // defaults; it is not a measurement of host free disk space. Mount access
    // is derived from Fs's real write policy, including immutable IPA assets.
    let mut result = fake_statfs();
    if !writable {
        result.f_flags |= MNT_RDONLY;
        result.f_bfree = 0;
        result.f_bavail = 0;
        result.f_ffree = 0;
    }
    result
}

fn statfs(env: &mut Environment, path: ConstPtr<u8>, buf: MutPtr<statfs>) -> i32 {
    // TODO: handle errno properly
    set_errno(env, 0);
    if buf.is_null() {
        set_errno(env, EFAULT);
        return -1;
    }

    let result = match statfs_inner(env, path) {
        Ok(statfs) => {
            env.mem.write(buf, statfs);
            0
        }
        Err(error) => {
            set_errno(env, error);
            -1
        }
    };

    log!(
        "TODO: statfs({:?}, {buf:?}) -> {result}",
        env.mem.cstr_at_utf8(path)
    );
    result
}

fn fstatfs(env: &mut Environment, fd: FileDescriptor, buf: MutPtr<statfs>) -> i32 {
    // TODO: handle errno properly
    set_errno(env, 0);
    if buf.is_null() {
        set_errno(env, EFAULT);
        return -1;
    }

    let result = if fd < 0
        || (!matches!(fd, STDIN_FILENO | STDOUT_FILENO | STDERR_FILENO)
            && !env.libc_state.posix_io.is_fd_open(fd))
    {
        set_errno(env, EBADF);
        -1
    } else {
        env.mem.write(buf, fake_statfs());
        0
    };

    log!("TODO: fstatfs({fd}, {buf:?}) -> {result}");
    result
}

pub const FUNCTIONS: FunctionExports =
    &[export_c_func!(statfs(_, _)), export_c_func!(fstatfs(_, _))];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readonly_root_is_queryable_and_missing_paths_fail() {
        let fs = Fs::new_fake_fs();
        let stats = statfs_for_path(&fs, "/").unwrap();
        let flags = stats.f_flags;
        let available = stats.f_bavail;
        assert_ne!(flags & MNT_RDONLY, 0);
        assert_eq!(available, 0);
        assert_eq!(statfs_for_path(&fs, "").unwrap_err(), ENOENT);
        assert_eq!(statfs_for_path(&fs, "/missing").unwrap_err(), ENOENT);
    }
    #[test]
    fn writable_defaults_and_readonly_mount_policy_are_distinct() {
        let writable = statfs_for_access(true);
        let readonly = statfs_for_access(false);
        let flags = writable.f_flags;
        let readonly_flags = readonly.f_flags;
        let available = writable.f_bavail;
        assert_eq!(flags & MNT_RDONLY, 0);
        assert_ne!(available, 0);
        assert_eq!(readonly_flags, flags | MNT_RDONLY);
        assert_eq!(readonly.f_fstypename, writable.f_fstypename);
        let reserved = readonly.f_reserved;
        assert_eq!(reserved, [0; 8]);
    }
}
