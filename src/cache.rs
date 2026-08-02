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

/// Serializes tests that mutate XDG_CACHE_HOME/HOME: env vars are
/// process-wide, so the cache.rs env tests and asnames::test_cache_round_trip
/// (which also resolves the cache dir) must not run concurrently.
#[cfg(test)]
pub(crate) static CACHE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// Restores XDG_CACHE_HOME/HOME on drop, including on panic, so a
    /// failing assertion cannot leak mutated env into other tests.
    struct RestoreEnv(Option<String>, Option<String>);
    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            match &self.0 {
                Some(v) => std::env::set_var("XDG_CACHE_HOME", v),
                None => std::env::remove_var("XDG_CACHE_HOME"),
            }
            match &self.1 {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    #[test]
    fn test_cache_dir_resolution() {
        let _lock = CACHE_ENV_LOCK.lock().unwrap();
        let _restore = RestoreEnv(
            std::env::var("XDG_CACHE_HOME").ok(),
            std::env::var("HOME").ok(),
        );

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

        std::env::remove_var("XDG_CACHE_HOME");
        std::env::set_var("HOME", "/tmp/bmpwatch-test-home");
        assert_eq!(
            cache_dir(),
            PathBuf::from("/tmp/bmpwatch-test-home/.cache/bmpwatch")
        );
    }
}
