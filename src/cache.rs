//! Cache directory helpers (XDG-compatible).
//!
//! All on-disk state (RPKI cache, AS-name cache, debug logs) lives under
//! `$XDG_CACHE_HOME/bmpwatch` (or `~/.cache/bmpwatch`). Debug logs must not
//! use predictable world-writable paths like `/tmp`: a local attacker could
//! pre-place a symlink there and redirect appends into arbitrary files.

use std::path::PathBuf;

/// `$XDG_CACHE_HOME/bmpwatch`, else `~/.cache/bmpwatch`, else `./bmpwatch`.
pub(crate) fn cache_dir() -> PathBuf {
    let base = if let Ok(dir) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(dir)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".cache")
    } else {
        PathBuf::from(".")
    };
    base.join("bmpwatch")
}

/// Path for a debug log under the cache dir, e.g. `cache_dir()/peering.log`.
pub(crate) fn debug_log_path(name: &str) -> PathBuf {
    cache_dir().join(format!("{name}.log"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_dir_uses_xdg() {
        let old_xdg = std::env::var("XDG_CACHE_HOME").ok();
        let old_home = std::env::var("HOME").ok();
        std::env::set_var("XDG_CACHE_HOME", "/tmp/bmpwatch-test-cache");
        std::env::set_var("HOME", "/tmp/bmpwatch-test-home");
        assert_eq!(
            cache_dir(),
            PathBuf::from("/tmp/bmpwatch-test-cache/bmpwatch")
        );
        assert_eq!(
            debug_log_path("peering"),
            PathBuf::from("/tmp/bmpwatch-test-cache/bmpwatch/peering.log")
        );
        // Restore
        match old_xdg {
            Some(v) => std::env::set_var("XDG_CACHE_HOME", v),
            None => std::env::remove_var("XDG_CACHE_HOME"),
        }
        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn test_cache_dir_falls_back_to_home() {
        let old_xdg = std::env::var("XDG_CACHE_HOME").ok();
        let old_home = std::env::var("HOME").ok();
        std::env::remove_var("XDG_CACHE_HOME");
        std::env::set_var("HOME", "/tmp/bmpwatch-test-home");
        assert_eq!(
            cache_dir(),
            PathBuf::from("/tmp/bmpwatch-test-home/.cache/bmpwatch")
        );
        match old_xdg {
            Some(v) => std::env::set_var("XDG_CACHE_HOME", v),
            None => std::env::remove_var("XDG_CACHE_HOME"),
        }
        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
