//! The shell scripts behind a privileged copy and move (`sudo sh -c ...`).

use std::ffi::OsString;

use super::PrivItem;

// Privileged copy and move. The paths are positional arguments of a fixed
// script, never spliced into it. `-T` makes the target THE name of the new
// item: without it an existing folder of that name swallows the item
// (conf/conf). A target that exists is refused unless a Trash slot was
// claimed for it, in which case the old item is moved there first and
// moved back if the new one does not arrive.

/// Refuse to touch an existing "$2".
pub(super) const REFUSE_EXISTING: &str = "if [ -e \"$2\" ] || [ -L \"$2\" ]; then \
    echo \"an item with this name already exists; nothing was overwritten\" >&2; exit 1; fi\n";

pub(super) fn sh(script: String, args: Vec<OsString>) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec!["sh".into(), "-c".into(), script.into(), "sh".into()];
    argv.extend(args);
    argv
}

/// "$1" source, "$2" target, "$3" staging name beside the target, "$4" the
/// old item's Trash slot (Replace only). The copy is built under the
/// staging name, so a failed copy leaves nothing under the real one.
pub(super) fn copy_argv(item: &PrivItem) -> Vec<OsString> {
    let staging = crate::copy_tree::temp_sibling(&item.target);
    let mut args: Vec<OsString> = vec![
        item.src.clone().into_os_string(),
        item.target.clone().into_os_string(),
        staging.into_os_string(),
    ];
    match &item.old_to_trash {
        None => sh(
            format!(
                "{REFUSE_EXISTING}\
                 cp -rT -- \"$1\" \"$3\" && mv -T -- \"$3\" \"$2\" && exit 0\n\
                 rm -rf -- \"$3\"\nexit 1"
            ),
            args,
        ),
        Some(slot) => {
            args.push(slot.trashed.clone().into_os_string());
            sh(
                "cp -rT -- \"$1\" \"$3\" || { rm -rf -- \"$3\"; exit 1; }\n\
                 mv -T -- \"$2\" \"$4\" || { rm -rf -- \"$3\"; exit 1; }\n\
                 mv -T -- \"$3\" \"$2\" && exit 0\n\
                 mv -T -- \"$4\" \"$2\"\nrm -rf -- \"$3\"\nexit 1"
                    .to_string(),
                args,
            )
        }
    }
}

/// A Replace move. "$1" source, "$2" target, "$3" staging name beside the
/// target, "$4" the old item's Trash slot.
///
/// The new item is complete under the staging name before the old one is
/// touched, as everywhere else. First a rename only (`--no-copy`: the same
/// filesystem, nothing can be half done). Otherwise copy to the staging
/// name, swap, and only then remove the source. Nothing here ever removes
/// the real name: across filesystems a plain `mv` copies everything, then
/// deletes its source, and can fail in that last step with the copy
/// complete; deleting "what the failed mv left" would delete the only full
/// copy.
pub(super) const MOVE_REPLACE: &str = "\
if mv --no-copy -T -- \"$1\" \"$3\" 2>/dev/null; then\n\
  mv -T -- \"$2\" \"$4\" || { mv -T -- \"$3\" \"$1\"; exit 1; }\n\
  mv -T -- \"$3\" \"$2\" && exit 0\n\
  mv -T -- \"$4\" \"$2\"; mv -T -- \"$3\" \"$1\"; exit 1\n\
fi\n\
cp -a -T -- \"$1\" \"$3\" || { rm -rf -- \"$3\"; exit 1; }\n\
mv -T -- \"$2\" \"$4\" || { rm -rf -- \"$3\"; exit 1; }\n\
mv -T -- \"$3\" \"$2\" || { mv -T -- \"$4\" \"$2\"; rm -rf -- \"$3\"; exit 1; }\n\
rm -rf --one-file-system -- \"$1\" && exit 0\n\
echo \"moved, but the original could not be removed completely\" >&2\nexit 1";

/// "$1" source, "$2" target; for a Replace see [`MOVE_REPLACE`].
pub(super) fn move_argv(item: &PrivItem) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        item.src.clone().into_os_string(),
        item.target.clone().into_os_string(),
    ];
    match &item.old_to_trash {
        None => sh(format!("{REFUSE_EXISTING}mv -T -- \"$1\" \"$2\""), args),
        Some(slot) => {
            args.push(crate::copy_tree::temp_sibling(&item.target).into_os_string());
            args.push(slot.trashed.clone().into_os_string());
            sh(MOVE_REPLACE.to_string(), args)
        }
    }
}
