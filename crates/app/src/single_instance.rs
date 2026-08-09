//! Stops a second copy putting a second dot in the bar.
//!
//! An advisory lock on a file, rather than a PID file, because the kernel drops
//! the lock when the process dies however it dies. A PID file survives a crash
//! and then refuses to start the app until someone deletes it by hand.

use std::fs::File;

/// Held for the lifetime of the process. Dropping it releases the lock.
pub struct InstanceGuard {
    _file: File,
}

/// Returns `None` when another instance already holds the lock.
pub fn acquire() -> Option<InstanceGuard> {
    let dir = gcloud_dot_core::paths::data_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let file = File::create(dir.join("instance.lock")).ok()?;
    // std's file locking, stable since 1.89, so no dependency is needed for it.
    file.try_lock().ok()?;
    Some(InstanceGuard { _file: file })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_is_exclusive_then_released() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.lock");

        let first = File::create(&path).unwrap();
        assert!(first.try_lock().is_ok());

        let second = File::create(&path).unwrap();
        assert!(
            second.try_lock().is_err(),
            "a second holder must be refused"
        );

        drop(first);
        let third = File::create(&path).unwrap();
        assert!(
            third.try_lock().is_ok(),
            "the lock must be free once the holder exits"
        );
    }
}
