use ignore::WalkBuilder;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::io;
use std::path::{Component, Path, PathBuf};

/// One visible line of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub path: PathBuf,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
}

/// The file tree on the right, modeled on Neo-tree: the folder's contents with expandable
/// folders, a selected row, and the operations behind its keys. It does the file work itself and
/// rebuilds its rows afterward. Directories come first, then files, each sorted without regard
/// to case. `.gitignore` is honored even outside a git repository.
#[derive(Debug)]
pub struct Tree {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    show_hidden: bool,
    rows: Vec<Row>,
    selected: usize,
    /// What `y` yanked, for `p` to copy.
    yanked: Option<PathBuf>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// The entries of `dir` that should show, as (path, is a directory), in display order.
fn list(dir: &Path, show_hidden: bool) -> Vec<(PathBuf, bool)> {
    let mut entries: Vec<(PathBuf, bool)> = WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(!show_hidden)
        .require_git(false)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.depth() == 1 && entry.file_name() != ".git" && entry.file_name() != ".DS_Store"
        })
        .map(|entry| {
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            (entry.into_path(), is_dir)
        })
        .collect();
    entries.sort_by_cached_key(|(path, is_dir)| {
        (
            !is_dir,
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase(),
        )
    });
    entries
}

/// A relative path made only of ordinary names, so it can't climb out of where it is joined.
fn plain_relative(name: &str) -> io::Result<&Path> {
    let path = Path::new(name);
    if name.is_empty() || !path.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(invalid("expected a name inside the folder"));
    }
    Ok(path)
}

fn copy_all(src: &Path, dst: &Path) -> io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_all(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}

/// `dir/name`, or if that exists, `dir/name copy.ext`, `dir/name copy 2.ext`, and so on.
fn unused_copy_name(dir: &Path, name: &OsStr) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path.file_stem().unwrap_or(name).to_string_lossy();
    let ext = path
        .extension()
        .map_or(String::new(), |e| format!(".{}", e.to_string_lossy()));
    (1..)
        .map(|n| {
            let copy = if n == 1 {
                " copy".to_string()
            } else {
                format!(" copy {n}")
            };
            dir.join(format!("{stem}{copy}{ext}"))
        })
        .find(|candidate| !candidate.exists())
        .unwrap()
}

impl Tree {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let mut tree = Self {
            root: root.into(),
            expanded: HashSet::new(),
            show_hidden: false,
            rows: Vec::new(),
            selected: 0,
            yanked: None,
        };
        tree.refresh();
        tree
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The folder the tree shows.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The index of the selected row.
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Selects the row at `index`, if there is one.
    pub fn select(&mut self, index: usize) {
        if index < self.rows.len() {
            self.selected = index;
        }
    }

    /// `path` as it is written in the prompts for copying and moving: from the tree's folder.
    pub fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .display()
            .to_string()
    }

    pub fn selected(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// Moves the selection by `delta` rows, stopping at the ends.
    pub fn move_selection(&mut self, delta: isize) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    /// `R`: reads the folder again, keeping the selection on the same path if it still exists.
    pub fn refresh(&mut self) {
        let keep = self.selected().map(|row| row.path.clone());
        self.rows.clear();
        let root = self.root.clone();
        self.fill(&root, 0);
        self.selected = keep
            .and_then(|path| self.rows.iter().position(|row| row.path == path))
            .unwrap_or(self.selected)
            .min(self.rows.len().saturating_sub(1));
    }

    fn fill(&mut self, dir: &Path, depth: usize) {
        for (path, is_dir) in list(dir, self.show_hidden) {
            let expanded = is_dir && self.expanded.contains(&path);
            self.rows.push(Row {
                path: path.clone(),
                depth,
                is_dir,
                expanded,
            });
            if expanded {
                self.fill(&path, depth + 1);
            }
        }
    }

    /// `H`: shows or hides dotfiles.
    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.refresh();
    }

    /// `<CR>`, `<Tab>`, and `o`: opens or closes a folder, or, on a file, returns its path for the
    /// app to open or preview.
    pub fn activate(&mut self) -> Option<PathBuf> {
        let row = self.selected()?.clone();
        if !row.is_dir {
            return Some(row.path);
        }
        if !self.expanded.remove(&row.path) {
            self.expanded.insert(row.path);
        }
        self.refresh();
        None
    }

    /// Opens every folder above `path` and selects it.
    fn reveal(&mut self, path: &Path) {
        for ancestor in path
            .ancestors()
            .skip(1)
            .take_while(|a| a.starts_with(&self.root) && *a != self.root)
        {
            self.expanded.insert(ancestor.to_path_buf());
        }
        self.refresh();
        if let Some(i) = self.rows.iter().position(|row| row.path == path) {
            self.selected = i;
        }
    }

    /// The folder new files go in: the selected folder, or the folder of the selected file.
    fn target_dir(&self) -> PathBuf {
        match self.selected() {
            Some(row) if row.is_dir => row.path.clone(),
            Some(row) => row.path.parent().unwrap_or(&self.root).to_path_buf(),
            None => self.root.clone(),
        }
    }

    /// `a`: makes a file, or a folder if `name` ends in `/`, in the target folder. Folders on the
    /// way are made too. Fails if it exists. `name` may not leave the folder.
    pub fn add(&mut self, name: &str) -> io::Result<PathBuf> {
        let path = self
            .target_dir()
            .join(plain_relative(name.trim_end_matches('/'))?);
        if path.exists() {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        if name.ends_with('/') {
            std::fs::create_dir_all(&path)?;
        } else {
            std::fs::create_dir_all(path.parent().unwrap_or(&self.root))?;
            std::fs::File::create_new(&path)?;
        }
        self.reveal(&path);
        Ok(path)
    }

    /// `d`: deletes the selected file, or folder with everything in it. It does not ask, so the
    /// caller should confirm first.
    pub fn delete(&mut self) -> io::Result<()> {
        let Some(row) = self.selected().cloned() else {
            return Ok(());
        };
        if row.is_dir {
            std::fs::remove_dir_all(&row.path)?;
        } else {
            std::fs::remove_file(&row.path)?;
        }
        self.expanded.retain(|p| !p.starts_with(&row.path));
        self.refresh();
        Ok(())
    }

    /// `r`: renames the selected entry within its folder.
    pub fn rename(&mut self, new_name: &str) -> io::Result<PathBuf> {
        let name = plain_relative(new_name)?;
        if name.components().count() != 1 {
            return Err(invalid("a new name can't hold a path"));
        }
        let from = self
            .selected()
            .ok_or_else(|| invalid("nothing selected"))?
            .path
            .clone();
        let to = from.with_file_name(name);
        self.relocate(&from, &to)
    }

    /// `m`: moves the selected entry to `dest`, relative to the tree's folder unless absolute.
    /// Folders on the way are made. Fails if `dest` exists.
    pub fn move_to(&mut self, dest: &str) -> io::Result<PathBuf> {
        let from = self
            .selected()
            .ok_or_else(|| invalid("nothing selected"))?
            .path
            .clone();
        let to = self.root.join(dest);
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)?;
        }
        self.relocate(&from, &to)
    }

    fn relocate(&mut self, from: &Path, to: &Path) -> io::Result<PathBuf> {
        if to.exists() {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        std::fs::rename(from, to)?;
        // Folders inside the moved one stay open.
        self.expanded = std::mem::take(&mut self.expanded)
            .into_iter()
            .map(|p| p.strip_prefix(from).map_or(p.clone(), |rest| to.join(rest)))
            .collect();
        self.reveal(to);
        Ok(to.to_path_buf())
    }

    /// `c`: copies the selected entry to `dest`, relative to the tree's folder unless absolute.
    /// Folders on the way are made. Fails if `dest` exists.
    pub fn copy_to(&mut self, dest: &str) -> io::Result<PathBuf> {
        let from = self
            .selected()
            .ok_or_else(|| invalid("nothing selected"))?
            .path
            .clone();
        let to = self.root.join(dest);
        if to.exists() {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        if to.starts_with(&from) {
            return Err(invalid("can't copy a folder into itself"));
        }
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)?;
        }
        copy_all(&from, &to)?;
        self.reveal(&to);
        Ok(to)
    }

    /// `y`: remembers the selected entry for `p` and returns its path, for the clipboard.
    pub fn yank(&mut self) -> Option<PathBuf> {
        self.yanked = Some(self.selected()?.path.clone());
        self.yanked.clone()
    }

    /// `p`: copies what `y` yanked into the target folder. A name already taken becomes `name copy`,
    /// `name copy 2`, and so on.
    pub fn paste(&mut self) -> io::Result<PathBuf> {
        let from = self
            .yanked
            .clone()
            .ok_or_else(|| invalid("nothing yanked"))?;
        let dir = self.target_dir();
        let to = unused_copy_name(&dir, from.file_name().unwrap_or_default());
        if to.starts_with(&from) {
            return Err(invalid("can't paste a folder into itself"));
        }
        copy_all(&from, &to)?;
        self.reveal(&to);
        Ok(to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder with a .gitignore and a few files and folders:
    /// chapters/{one.dj,two.dj}, a.dj, Notes.dj, .hidden.dj, skip.tmp (ignored), .git/.
    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("bwa_tree_{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("chapters")).unwrap();
        for file in [
            ".gitignore",
            "chapters/one.dj",
            "chapters/two.dj",
            "a.dj",
            "Notes.dj",
            ".hidden.dj",
            "skip.tmp",
        ] {
            std::fs::write(
                root.join(file),
                if file == ".gitignore" { "*.tmp\n" } else { "x" },
            )
            .unwrap();
        }
        root
    }

    /// The rows as indented names, folders with a trailing slash; the selected one is marked.
    fn show(tree: &Tree) -> Vec<String> {
        tree.rows()
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let name = row.path.file_name().unwrap().to_string_lossy();
                let slash = if row.is_dir { "/" } else { "" };
                let mark = if i == tree.selected { "*" } else { "" };
                format!("{}{name}{slash}{mark}", "  ".repeat(row.depth))
            })
            .collect()
    }

    fn select(tree: &mut Tree, name: &str) {
        tree.selected = tree
            .rows
            .iter()
            .position(|r| r.path.ends_with(name))
            .unwrap();
    }

    #[test]
    fn lists_folders_first_sorted_and_hides_dotfiles_and_ignored_files() {
        let root = fixture("list");
        let mut tree = Tree::new(&root);
        assert_eq!(show(&tree), ["chapters/*", "a.dj", "Notes.dj"]);
        // H shows dotfiles, but .git never shows and .gitignore still applies.
        tree.toggle_hidden();
        assert_eq!(
            show(&tree),
            ["chapters/*", ".gitignore", ".hidden.dj", "a.dj", "Notes.dj"]
        );
        tree.toggle_hidden();
        assert_eq!(show(&tree).len(), 3);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn folders_open_and_close_and_files_are_returned() {
        let root = fixture("open");
        let mut tree = Tree::new(&root);
        assert_eq!(tree.activate(), None);
        assert_eq!(
            show(&tree),
            ["chapters/*", "  one.dj", "  two.dj", "a.dj", "Notes.dj"]
        );
        tree.move_selection(2);
        assert_eq!(tree.activate(), Some(root.join("chapters/two.dj")));
        tree.move_selection(99);
        assert_eq!(tree.selected().unwrap().path, root.join("Notes.dj"));
        tree.move_selection(-99);
        assert_eq!(tree.activate(), None);
        assert_eq!(show(&tree), ["chapters/*", "a.dj", "Notes.dj"]);
        // R picks up changes made outside, and keeps the selection.
        std::fs::write(root.join("b.dj"), "x").unwrap();
        tree.move_selection(1);
        tree.refresh();
        assert_eq!(show(&tree), ["chapters/", "a.dj*", "b.dj", "Notes.dj"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn add_makes_files_and_folders_where_the_selection_is() {
        let root = fixture("add");
        let mut tree = Tree::new(&root);
        // On a folder, it goes inside.
        assert_eq!(
            tree.add("three.dj").unwrap(),
            root.join("chapters/three.dj")
        );
        assert_eq!(
            show(&tree),
            [
                "chapters/",
                "  one.dj",
                "  three.dj*",
                "  two.dj",
                "a.dj",
                "Notes.dj"
            ]
        );
        // On a file, next to it. A trailing slash makes a folder, and folders on the way are made.
        tree.add("more/deep/four.dj").unwrap();
        tree.add("empty/").unwrap();
        assert!(root.join("chapters/more/deep/four.dj").is_file());
        // The new file is selected, so this one goes next to it.
        assert!(root.join("chapters/more/deep/empty").is_dir());
        // Bad names and existing ones fail without changing anything.
        select(&mut tree, "one.dj");
        for name in ["", "/", "../out.dj", "a/../../out.dj", "/abs.dj", "one.dj"] {
            assert!(tree.add(name).is_err(), "{name:?}");
        }
        assert!(!root.join("out.dj").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rename_and_delete() {
        let root = fixture("rename");
        let mut tree = Tree::new(&root);
        tree.activate();
        // A renamed folder keeps its open state, and the selection follows the rename.
        assert_eq!(tree.rename("parts").unwrap(), root.join("parts"));
        assert_eq!(
            show(&tree),
            ["parts/*", "  one.dj", "  two.dj", "a.dj", "Notes.dj"]
        );
        select(&mut tree, "one.dj");
        tree.rename("1.dj").unwrap();
        assert_eq!(
            show(&tree),
            ["parts/", "  1.dj*", "  two.dj", "a.dj", "Notes.dj"]
        );
        for bad in ["", "..", "x/y", "two.dj", "/etc/x"] {
            assert!(tree.rename(bad).is_err(), "{bad:?}");
        }
        tree.delete().unwrap();
        assert!(!root.join("parts/1.dj").exists());
        assert_eq!(show(&tree), ["parts/", "  two.dj*", "a.dj", "Notes.dj"]);
        // Deleting a folder takes what is in it.
        select(&mut tree, "parts");
        tree.delete().unwrap();
        assert!(!root.join("parts").exists());
        assert_eq!(show(&tree), ["a.dj*", "Notes.dj"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn move_and_copy_to_a_path() {
        let root = fixture("moves");
        let mut tree = Tree::new(&root);
        select(&mut tree, "a.dj");
        assert_eq!(
            tree.copy_to("backup/a.dj").unwrap(),
            root.join("backup/a.dj")
        );
        assert!(root.join("a.dj").exists() && root.join("backup/a.dj").exists());
        assert_eq!(
            show(&tree),
            ["backup/", "  a.dj*", "chapters/", "a.dj", "Notes.dj"]
        );
        // Moving takes it away from where it was, and needs a free destination.
        select(&mut tree, "Notes.dj");
        assert!(tree.move_to("a.dj").is_err());
        tree.move_to("backup/notes/Notes.dj").unwrap();
        assert!(!root.join("Notes.dj").exists());
        assert!(root.join("backup/notes/Notes.dj").exists());
        // A folder moves with its contents, and can't be copied into itself.
        select(&mut tree, "chapters");
        assert!(tree.copy_to("chapters/again").is_err());
        tree.move_to("backup/chapters").unwrap();
        assert!(root.join("backup/chapters/two.dj").exists());
        assert!(!root.join("chapters").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn yank_and_paste_copy_into_the_target_folder() {
        let root = fixture("paste");
        let mut tree = Tree::new(&root);
        assert!(tree.paste().is_err());
        select(&mut tree, "a.dj");
        assert_eq!(tree.yank(), Some(root.join("a.dj")));
        // On a file it pastes next to it, and taken names get "copy".
        assert_eq!(tree.paste().unwrap(), root.join("a copy.dj"));
        assert_eq!(tree.paste().unwrap(), root.join("a copy 2.dj"));
        // On a folder it pastes inside.
        select(&mut tree, "chapters");
        assert_eq!(tree.paste().unwrap(), root.join("chapters/a.dj"));
        // A folder copies whole, and not into itself.
        select(&mut tree, "chapters");
        assert_eq!(tree.yank(), Some(root.join("chapters")));
        assert!(tree.paste().is_err());
        select(&mut tree, "Notes.dj");
        assert_eq!(tree.paste().unwrap(), root.join("chapters copy"));
        assert!(root.join("chapters copy/one.dj").exists());
        assert!(root.join("chapters copy/a.dj").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
