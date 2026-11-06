//! The one-value files under `/sys` and `/proc`: read as trimmed text or
//! a number, written whole.

use std::path::Path;

/// The file's text without its trailing newline, if it can be read.
pub fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

pub fn read_u64(path: &Path) -> Option<u64> {
    read(path)?.parse().ok()
}

/// Write `value`, saying which file refused it and why.
pub fn write(path: &Path, value: &str) -> Result<(), String> {
    std::fs::write(path, value).map_err(|e| format!("{}: {e}", path.display()))
}

/// The words of a space-separated list file: a driver's governors.
pub fn words(path: &Path) -> Vec<String> {
    read(path).map(|s| s.split_whitespace().map(str::to_owned).collect()).unwrap_or_default()
}

/// How many CPUs a sysfs list like `0-1,4` names.
pub fn count_list(list: &str) -> usize {
    list.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| match part.split_once('-') {
            Some((a, b)) => match (a.parse::<usize>(), b.parse::<usize>()) {
                (Ok(a), Ok(b)) => b.saturating_sub(a) + 1,
                _ => 0,
            },
            None => 1,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cpu_list_is_counted_by_its_ranges() {
        assert_eq!(count_list("0-1"), 2);
        assert_eq!(count_list("0,1"), 2);
        assert_eq!(count_list("3"), 1);
        assert_eq!(count_list("0-1,4,8-11"), 7);
        assert_eq!(count_list(""), 0);
        assert_eq!(count_list("x-y"), 0);
    }
}
