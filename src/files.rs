use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

/// Lists files under `root`, skipping anything `.gitignore` excludes. Sorted for stable output.
pub fn list_files(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkBuilder::new(root)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
        .map(|entry| entry.into_path())
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respects_gitignore() {
        let root = std::env::temp_dir().join("bwa_files_gitignore");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "drafts/\n").unwrap();
        std::fs::create_dir_all(root.join("drafts")).unwrap();
        std::fs::write(root.join("drafts/skip.md"), "x").unwrap();
        std::fs::write(root.join("chapter1.md"), "x").unwrap();

        let names: Vec<_> = list_files(&root)
            .iter()
            .filter_map(|p| p.file_name()?.to_str().map(String::from))
            .collect();
        assert!(names.contains(&"chapter1.md".to_string()));
        assert!(!names.contains(&"skip.md".to_string()));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
