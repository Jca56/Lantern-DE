//! A folder's custom icon and colour, kept on the folder itself as
//! `user.lantern.*` extended attributes.

use std::path::Path;

const XATTR_FOLDER_COLOR: &str = "user.lantern.folder_color";
const XATTR_FOLDER_ICON: &str = "user.lantern.folder_icon";

/// A folder's custom icon path and colour, straight from its attributes.
/// Two syscalls, each a device round-trip on a slow mount: for listing and
/// worker threads, never for drawing (entries carry the result).
pub fn read_folder_attrs(path: &Path) -> (Option<String>, Option<String>) {
    (
        read_xattr(path, XATTR_FOLDER_ICON),
        read_xattr(path, XATTR_FOLDER_COLOR),
    )
}

/// Set a custom icon path xattr on a directory.
pub fn set_folder_icon(path: &Path, icon_path: &str) {
    write_xattr(path, XATTR_FOLDER_ICON, icon_path);
}

/// Remove a custom folder icon xattr — reverts to the default folder icon.
pub fn clear_folder_icon(path: &Path) {
    remove_xattr(path, XATTR_FOLDER_ICON);
}

/// Set a folder color xattr on a directory path.
pub fn set_folder_color(path: &Path, color: &str) {
    write_xattr(path, XATTR_FOLDER_COLOR, color);
}

fn read_xattr(path: &Path, attr: &str) -> Option<String> {
    use std::ffi::CString;
    let c_path = CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    let c_name = CString::new(attr).ok()?;
    let mut buf = [0u8; 512];
    let len = unsafe {
        libc::getxattr(
            c_path.as_ptr(),
            c_name.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len(),
        )
    };
    if len > 0 {
        Some(String::from_utf8_lossy(&buf[..len as usize]).to_string())
    } else {
        None
    }
}

fn write_xattr(path: &Path, attr: &str, value: &str) {
    use std::ffi::CString;
    let Some(c_path) = CString::new(path.as_os_str().as_encoded_bytes()).ok() else {
        return;
    };
    let Some(c_name) = CString::new(attr).ok() else {
        return;
    };
    unsafe {
        libc::setxattr(
            c_path.as_ptr(),
            c_name.as_ptr(),
            value.as_ptr() as *const libc::c_void,
            value.len(),
            0,
        );
    }
}

fn remove_xattr(path: &Path, attr: &str) {
    use std::ffi::CString;
    let Some(c_path) = CString::new(path.as_os_str().as_encoded_bytes()).ok() else {
        return;
    };
    let Some(c_name) = CString::new(attr).ok() else {
        return;
    };
    unsafe {
        libc::removexattr(c_path.as_ptr(), c_name.as_ptr());
    }
}
