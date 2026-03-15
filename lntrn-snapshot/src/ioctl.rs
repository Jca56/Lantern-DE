///! Raw btrfs ioctl bindings — struct layouts matching linux/btrfs.h

// ── Constants ──────────────────────────────────────────────────────

pub const BTRFS_PATH_NAME_MAX: usize = 4087;
pub const BTRFS_SUBVOL_NAME_MAX: usize = 4039;
pub const BTRFS_SEARCH_ARGS_BUFSIZE: usize = 3992;

// Ioctl numbers (precomputed from _IOW/_IOWR macros)
pub const BTRFS_IOC_SUBVOL_CREATE: libc::c_ulong = 0x5000_940E;
pub const BTRFS_IOC_SNAP_DESTROY: libc::c_ulong = 0x5000_940F;
pub const BTRFS_IOC_TREE_SEARCH: libc::c_ulong = 0xD000_9411;
pub const BTRFS_IOC_SNAP_CREATE_V2: libc::c_ulong = 0x5000_9417;

// Snapshot flags
pub const BTRFS_SUBVOL_RDONLY: u64 = 1 << 1;

// Tree search constants
pub const BTRFS_ROOT_TREE_OBJECTID: u64 = 1;
pub const BTRFS_FIRST_FREE_OBJECTID: u64 = 256;
pub const BTRFS_ROOT_ITEM_KEY: u32 = 132;
pub const BTRFS_ROOT_BACKREF_KEY: u32 = 144;

// ── Structs ────────────────────────────────────────────────────────

/// Used by SNAP_DESTROY and SUBVOL_CREATE (4096 bytes total)
#[repr(C)]
pub struct BtrfsIoctlVolArgs {
    pub fd: i64,
    pub name: [u8; BTRFS_PATH_NAME_MAX + 1], // 4088
}

impl BtrfsIoctlVolArgs {
    pub fn new() -> Self {
        Self {
            fd: 0,
            name: [0u8; BTRFS_PATH_NAME_MAX + 1],
        }
    }

    pub fn set_name(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let len = bytes.len().min(BTRFS_PATH_NAME_MAX);
        self.name[..len].copy_from_slice(&bytes[..len]);
        self.name[len] = 0;
    }
}

/// Used by SNAP_CREATE_V2 (4096 bytes total)
#[repr(C)]
pub struct BtrfsIoctlVolArgsV2 {
    pub fd: i64,
    pub transid: u64,
    pub flags: u64,
    pub unused: [u64; 4],
    pub name: [u8; BTRFS_SUBVOL_NAME_MAX + 1], // 4040
}

impl BtrfsIoctlVolArgsV2 {
    pub fn new() -> Self {
        Self {
            fd: 0,
            transid: 0,
            flags: 0,
            unused: [0u64; 4],
            name: [0u8; BTRFS_SUBVOL_NAME_MAX + 1],
        }
    }

    pub fn set_name(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let len = bytes.len().min(BTRFS_SUBVOL_NAME_MAX);
        self.name[..len].copy_from_slice(&bytes[..len]);
        self.name[len] = 0;
    }
}

/// Search key for TREE_SEARCH (104 bytes)
#[repr(C)]
#[derive(Clone)]
pub struct BtrfsIoctlSearchKey {
    pub tree_id: u64,
    pub min_objectid: u64,
    pub max_objectid: u64,
    pub min_offset: u64,
    pub max_offset: u64,
    pub min_transid: u64,
    pub max_transid: u64,
    pub min_type: u32,
    pub max_type: u32,
    pub nr_items: u32,
    pub unused: u32,
    pub unused1: u64,
    pub unused2: u64,
    pub unused3: u64,
    pub unused4: u64,
}

/// Search args for TREE_SEARCH (4096 bytes total)
#[repr(C)]
pub struct BtrfsIoctlSearchArgs {
    pub key: BtrfsIoctlSearchKey,
    pub buf: [u8; BTRFS_SEARCH_ARGS_BUFSIZE], // 3992
}

impl BtrfsIoctlSearchArgs {
    pub fn new() -> Self {
        Self {
            key: BtrfsIoctlSearchKey {
                tree_id: 0,
                min_objectid: 0,
                max_objectid: 0,
                min_offset: 0,
                max_offset: 0,
                min_transid: 0,
                max_transid: 0,
                min_type: 0,
                max_type: 0,
                nr_items: 0,
                unused: 0,
                unused1: 0,
                unused2: 0,
                unused3: 0,
                unused4: 0,
            },
            buf: [0u8; BTRFS_SEARCH_ARGS_BUFSIZE],
        }
    }
}

/// Search result header (32 bytes, appears in search buf)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsIoctlSearchHeader {
    pub transid: u64,
    pub objectid: u64,
    pub offset: u64,
    pub item_type: u32,
    pub len: u32,
}

/// Root backref data — followed by `name_len` bytes of name
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsRootRef {
    pub dirid: u64,
    pub sequence: u64,
    pub name_len: u16,
}

impl BtrfsRootRef {
    /// Read the name bytes that follow this struct in the search buffer
    pub unsafe fn name_from_buf<'a>(&self, ptr: *const u8) -> &'a [u8] {
        let name_start = ptr.add(std::mem::size_of::<BtrfsRootRef>());
        std::slice::from_raw_parts(name_start, self.name_len as usize)
    }
}

/// btrfs_timespec (on-disk, packed: 12 bytes)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsTimespec {
    pub sec: u64,  // le64
    pub nsec: u32, // le32
}

// ── Ioctl wrappers ─────────────────────────────────────────────────

/// Create a read-only snapshot of `src_fd` at `dst_dir_fd`/`name`
pub unsafe fn snap_create(
    dst_dir_fd: libc::c_int,
    src_fd: libc::c_int,
    name: &str,
    readonly: bool,
) -> Result<(), std::io::Error> {
    let mut args = BtrfsIoctlVolArgsV2::new();
    args.fd = src_fd as i64;
    args.set_name(name);
    if readonly {
        args.flags = BTRFS_SUBVOL_RDONLY;
    }
    let ret = libc::ioctl(dst_dir_fd, BTRFS_IOC_SNAP_CREATE_V2, &mut args);
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Delete a snapshot/subvolume by name under `parent_fd`
pub unsafe fn snap_destroy(
    parent_fd: libc::c_int,
    name: &str,
) -> Result<(), std::io::Error> {
    let mut args = BtrfsIoctlVolArgs::new();
    args.set_name(name);
    let ret = libc::ioctl(parent_fd, BTRFS_IOC_SNAP_DESTROY, &mut args);
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Create a subvolume (e.g. .snapshots) under `parent_fd`
pub unsafe fn subvol_create(
    parent_fd: libc::c_int,
    name: &str,
) -> Result<(), std::io::Error> {
    let mut args = BtrfsIoctlVolArgs::new();
    args.set_name(name);
    let ret = libc::ioctl(parent_fd, BTRFS_IOC_SUBVOL_CREATE, &mut args);
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Info about a discovered subvolume
#[derive(Debug, Clone)]
pub struct SubvolInfo {
    pub id: u64,
    pub parent_id: u64,
    pub name: String,
}

/// List all subvolumes by searching ROOT_BACKREF_KEY entries
pub unsafe fn list_subvolumes(
    fs_fd: libc::c_int,
) -> Result<Vec<SubvolInfo>, std::io::Error> {
    let mut results = Vec::new();
    let mut args = BtrfsIoctlSearchArgs::new();

    args.key.tree_id = BTRFS_ROOT_TREE_OBJECTID;
    args.key.min_objectid = BTRFS_FIRST_FREE_OBJECTID;
    args.key.max_objectid = u64::MAX;
    args.key.min_type = BTRFS_ROOT_BACKREF_KEY;
    args.key.max_type = BTRFS_ROOT_BACKREF_KEY;
    args.key.min_offset = 0;
    args.key.max_offset = u64::MAX;
    args.key.min_transid = 0;
    args.key.max_transid = u64::MAX;
    args.key.nr_items = 4096;

    loop {
        let ret = libc::ioctl(fs_fd, BTRFS_IOC_TREE_SEARCH, &mut args);
        if ret < 0 {
            return Err(std::io::Error::last_os_error());
        }

        if args.key.nr_items == 0 {
            break;
        }

        let mut offset = 0usize;
        for _ in 0..args.key.nr_items {
            if offset + std::mem::size_of::<BtrfsIoctlSearchHeader>() > BTRFS_SEARCH_ARGS_BUFSIZE
            {
                break;
            }

            let header = &*(args.buf.as_ptr().add(offset)
                as *const BtrfsIoctlSearchHeader);
            offset += std::mem::size_of::<BtrfsIoctlSearchHeader>();

            if header.item_type == BTRFS_ROOT_BACKREF_KEY
                && (header.len as usize) >= std::mem::size_of::<BtrfsRootRef>()
            {
                let data_ptr = args.buf.as_ptr().add(offset);
                let root_ref = &*(data_ptr as *const BtrfsRootRef);
                let name_bytes = root_ref.name_from_buf(data_ptr);
                let name = String::from_utf8_lossy(name_bytes).into_owned();

                results.push(SubvolInfo {
                    id: header.objectid,
                    parent_id: header.offset,
                    name,
                });
            }

            offset += header.len as usize;

            // Update search key for next iteration
            args.key.min_objectid = header.objectid;
            args.key.min_type = header.item_type;
            args.key.min_offset = header.offset + 1;
        }

        args.key.nr_items = 4096;
    }

    Ok(results)
}
