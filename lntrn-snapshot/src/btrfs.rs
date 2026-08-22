///! Raw btrfs ioctls — CoW snapshots without shelling out to btrfs-progs.
use std::ffi::CString;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// `statfs` filesystem magic for btrfs.
const BTRFS_SUPER_MAGIC: i64 = 0x9123_683E;

/// Subvolume root directories always have this inode number.
const BTRFS_FIRST_FREE_OBJECTID: u64 = 256;

const BTRFS_IOCTL_MAGIC: u64 = 0x94;
const BTRFS_SUBVOL_RDONLY: u64 = 1 << 1;
const BTRFS_SUBVOL_NAME_MAX: usize = 4039;

/// Mirror of `struct btrfs_ioctl_vol_args_v2` (must be exactly 4096 bytes).
#[repr(C)]
struct VolArgsV2 {
    fd: i64,
    transid: u64,
    flags: u64,
    unused: [u64; 4],
    name: [u8; BTRFS_SUBVOL_NAME_MAX + 1],
}

const _: () = assert!(std::mem::size_of::<VolArgsV2>() == 4096);

/// `_IOW(BTRFS_IOCTL_MAGIC, nr, struct btrfs_ioctl_vol_args_v2)`
const fn iow_v2(nr: u64) -> libc::c_ulong {
    ((1u64 << 30) | (4096 << 16) | (BTRFS_IOCTL_MAGIC << 8) | nr) as libc::c_ulong
}

const BTRFS_IOC_SNAP_CREATE_V2: libc::c_ulong = iow_v2(23);
const BTRFS_IOC_SNAP_DESTROY_V2: libc::c_ulong = iow_v2(63);

/// True when `path` lives on a btrfs filesystem.
pub fn is_btrfs(path: &Path) -> bool {
    let Ok(c) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    unsafe { libc::statfs(c.as_ptr(), &mut st) == 0 && st.f_type as i64 == BTRFS_SUPER_MAGIC }
}

/// True when `path` is the root directory of a btrfs subvolume.
pub fn is_subvolume(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    meta.is_dir() && meta.ino() == BTRFS_FIRST_FREE_OBJECTID && is_btrfs(path)
}

fn open_dir(path: &Path) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY)
        .open(path)
}

fn split_parent_name(path: &Path) -> io::Result<(&Path, &std::ffi::OsStr)> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    if name.as_bytes().len() > BTRFS_SUBVOL_NAME_MAX {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "name too long"));
    }
    Ok((parent, name))
}

fn vol_args(fd: i64, flags: u64, name: &std::ffi::OsStr) -> VolArgsV2 {
    let mut args: VolArgsV2 = unsafe { std::mem::zeroed() };
    args.fd = fd;
    args.flags = flags;
    let bytes = name.as_bytes();
    args.name[..bytes.len()].copy_from_slice(bytes);
    args
}

/// Create a CoW snapshot of the subvolume at `src` at path `dest`.
/// `dest`'s parent must exist on the same btrfs filesystem.
pub fn snapshot(src: &Path, dest: &Path, readonly: bool) -> io::Result<()> {
    let (parent, name) = split_parent_name(dest)?;
    let src_dir = open_dir(src)?;
    let dst_dir = open_dir(parent)?;
    let flags = if readonly { BTRFS_SUBVOL_RDONLY } else { 0 };
    let args = vol_args(src_dir.as_raw_fd() as i64, flags, name);
    let r = unsafe { libc::ioctl(dst_dir.as_raw_fd(), BTRFS_IOC_SNAP_CREATE_V2, &args) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Destroy the subvolume at `path` (works on read-only snapshots too).
/// Fails with `EBUSY` while the subvolume is mounted.
pub fn delete_subvolume(path: &Path) -> io::Result<()> {
    let (parent, name) = split_parent_name(path)?;
    let dir = open_dir(parent)?;
    let args = vol_args(0, 0, name);
    let r = unsafe { libc::ioctl(dir.as_raw_fd(), BTRFS_IOC_SNAP_DESTROY_V2, &args) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Where a btrfs mount point comes from: the backing device and the
/// subvolume path within the filesystem (e.g. "/@" for the usual root).
#[derive(Debug, Clone)]
pub struct MountInfo {
    pub device: String,
    /// Subvolume path inside the filesystem, e.g. "/@" or "/@home".
    /// "/" means the top-level subvolume is mounted directly.
    pub subvol: String,
}

/// Look up the btrfs mount entry whose mount point is exactly `mount_point`.
pub fn mount_info_for(mount_point: &Path) -> Option<MountInfo> {
    let content = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    let want = mount_point.to_str()?;
    for line in content.lines() {
        // <id> <parent> <maj:min> <subvol-root> <mount-point> <opts> ... - <fstype> <device> <super-opts>
        let Some((pre, post)) = line.split_once(" - ") else {
            continue;
        };
        let pre_fields: Vec<&str> = pre.split(' ').collect();
        if pre_fields.len() < 5 {
            continue;
        }
        if unescape(pre_fields[4]) != want {
            continue;
        }
        let post_fields: Vec<&str> = post.split(' ').collect();
        if post_fields.len() < 2 || post_fields[0] != "btrfs" {
            continue;
        }
        return Some(MountInfo {
            device: unescape(post_fields[1]),
            subvol: unescape(pre_fields[3]),
        });
    }
    None
}

/// The closest mount point containing `path` (longest matching prefix).
/// Used to verify a snapshot dir lives on a different mount than the
/// subvolume being rolled back — otherwise the swap would strand it.
pub fn nearest_mount_point(path: &Path) -> Option<PathBuf> {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let content = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    let mut best: Option<PathBuf> = None;
    for line in content.lines() {
        let Some((pre, _)) = line.split_once(" - ") else {
            continue;
        };
        let pre_fields: Vec<&str> = pre.split(' ').collect();
        if pre_fields.len() < 5 {
            continue;
        }
        let mp = PathBuf::from(unescape(pre_fields[4]));
        if resolved.starts_with(&mp)
            && best
                .as_ref()
                .map_or(true, |b| mp.components().count() > b.components().count())
        {
            best = Some(mp);
        }
    }
    best
}

/// Octal escapes used in /proc/self/mountinfo fields.
fn unescape(s: &str) -> String {
    s.replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

/// Mount the top-level subvolume (subvolid=5) of `device` at `at`,
/// creating the mount point if needed.
pub fn mount_toplevel(device: &str, at: &Path) -> io::Result<()> {
    std::fs::create_dir_all(at)?;
    let src = CString::new(device)?;
    let tgt = CString::new(at.as_os_str().as_bytes())?;
    let fstype = CString::new("btrfs")?;
    let data = CString::new("subvolid=5")?;
    let r = unsafe {
        libc::mount(
            src.as_ptr(),
            tgt.as_ptr(),
            fstype.as_ptr(),
            0,
            data.as_ptr() as *const libc::c_void,
        )
    };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn unmount(at: &Path) -> io::Result<()> {
    let tgt = CString::new(at.as_os_str().as_bytes())?;
    let r = unsafe { libc::umount2(tgt.as_ptr(), 0) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Scoped top-level mount: mounts subvolid=5 on creation, unmounts on drop.
pub struct ToplevelMount {
    pub path: PathBuf,
}

impl ToplevelMount {
    pub fn new(device: &str) -> io::Result<Self> {
        let path = PathBuf::from("/run/lntrn-snapshot/toplevel");
        mount_toplevel(device, &path)?;
        Ok(Self { path })
    }

    /// Resolve a subvolume path from mountinfo (e.g. "/@") to a real path
    /// under this mount.
    pub fn subvol_path(&self, subvol: &str) -> PathBuf {
        self.path.join(subvol.trim_start_matches('/'))
    }
}

impl Drop for ToplevelMount {
    fn drop(&mut self) {
        let _ = unmount(&self.path);
    }
}
