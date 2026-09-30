use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;

/// Calls a function when a file changes on disk. It watches the file's folder rather than the file
/// itself, since many editors save by writing a new file and renaming it into place, which a
/// watch on the old file would lose. Dropping it stops the watching.
pub struct FileWatcher {
    _watcher: RecommendedWatcher,
}

impl FileWatcher {
    /// `on_change` runs on another thread, for each event that touches `path`. It may run more
    /// than once for one save, so it should check whether anything really changed.
    pub fn new(path: &Path, on_change: impl Fn() + Send + 'static) -> notify::Result<Self> {
        let path = std::fs::canonicalize(path)?;
        let folder = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            if let Ok(event) = event
                && event.paths.iter().any(|p| p == &path)
            {
                on_change();
            }
        })?;
        watcher.watch(&folder, RecursiveMode::NonRecursive)?;
        Ok(Self { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn reports_changes_to_its_file_and_only_its_file() {
        let dir = std::env::temp_dir().join("bwa_watch");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (watched, other) = (dir.join("book.dj"), dir.join("other.dj"));
        std::fs::write(&watched, "one").unwrap();
        std::fs::write(&other, "one").unwrap();
        let (sender, receiver) = mpsc::channel();
        let Ok(_watcher) = FileWatcher::new(&watched, move || {
            let _ = sender.send(());
        }) else {
            // Some sandboxes can't watch files at all; that is not this code's fault.
            return;
        };
        // Give the watch a moment to start, then change the other file: no news.
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(&other, "two").unwrap();
        assert!(receiver.recv_timeout(Duration::from_millis(500)).is_err());
        // A write in place, and a save by renaming a new file over it, both count.
        std::fs::write(&watched, "two").unwrap();
        assert!(receiver.recv_timeout(Duration::from_secs(3)).is_ok());
        while receiver.try_recv().is_ok() {}
        std::fs::write(dir.join("tmp.dj"), "three").unwrap();
        std::fs::rename(dir.join("tmp.dj"), &watched).unwrap();
        assert!(receiver.recv_timeout(Duration::from_secs(3)).is_ok());
        drop(_watcher);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
