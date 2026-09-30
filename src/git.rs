//! Git, by way of the `git` program: what changed, and the commands that stage, commit and share it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};

/// How a changed file or folder is shown. The order is which wins when a folder holds several.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tone {
    Untracked,
    Added,
    Modified,
    Deleted,
    Conflict,
}

/// The two status letters git gives a file: what is staged, and what is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub index: char,
    pub worktree: char,
}

impl Entry {
    pub fn tone(&self) -> Tone {
        match (self.index, self.worktree) {
            ('?', _) => Tone::Untracked,
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => Tone::Conflict,
            ('D', _) | (_, 'D') => Tone::Deleted,
            ('A', _) | ('R', _) | ('C', _) => Tone::Added,
            _ => Tone::Modified,
        }
    }

    /// The letter shown beside it, as in VS Code.
    pub fn letter(&self) -> char {
        match (self.tone(), self.index) {
            (Tone::Untracked, _) => 'U',
            (Tone::Conflict, _) => 'C',
            (Tone::Deleted, _) => 'D',
            (Tone::Added, 'R') => 'R',
            (Tone::Added, _) => 'A',
            _ => 'M',
        }
    }

    pub fn staged(&self) -> bool {
        !matches!(self.index, ' ' | '?' | 'U')
    }
}

/// What `git status` says about a repository.
#[derive(Debug, Default, PartialEq)]
pub struct Status {
    /// The branch, or `HEAD` when detached.
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
    /// Every changed file, by absolute path.
    pub files: HashMap<PathBuf, Entry>,
    /// Every folder with changes in it, and the strongest change.
    pub dirs: HashMap<PathBuf, Tone>,
}

impl Status {
    /// How many files have changes staged.
    pub fn staged(&self) -> usize {
        self.files.values().filter(|e| e.staged()).count()
    }

    /// The tone of a file or of a folder that has changes in it.
    pub fn tone(&self, path: &Path) -> Option<Tone> {
        self.files
            .get(path)
            .map(Entry::tone)
            .or_else(|| self.dirs.get(path).copied())
    }
}

/// A repository, by the folder at its top.
#[derive(Debug, Clone, PartialEq)]
pub struct Repo {
    pub root: PathBuf,
}

/// The first line of what a failed command printed.
fn first_line(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("git failed")
        .to_string()
}

impl Repo {
    /// The repository that holds `dir`, if there is one and `git` is installed.
    pub fn discover(dir: &Path) -> Option<Repo> {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "--show-toplevel"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())?;
        let top = String::from_utf8(out.stdout).ok()?;
        let root = std::fs::canonicalize(top.trim()).ok()?;
        Some(Repo { root })
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new("git");
        // Never wait for a person to type a password nobody can see.
        command
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null());
        command
    }

    /// Runs git, giving what it printed or the first line of its complaint.
    fn run(&self, args: &[&str]) -> Result<String, String> {
        let out = self.command(args).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let said = if out.stderr.is_empty() {
                &out.stdout
            } else {
                &out.stderr
            };
            Err(first_line(said))
        }
    }

    /// Runs git for `paths`, which are given as absolute paths in the repository.
    fn run_on(&self, args: &[&str], paths: &[PathBuf]) -> Result<String, String> {
        let mut command = self.command(args);
        command.arg("--");
        for path in paths {
            // The top folder itself is `.`.
            let rel = path.strip_prefix(&self.root).unwrap_or(path);
            command.arg(if rel.as_os_str().is_empty() {
                Path::new(".")
            } else {
                rel
            });
        }
        let out = command.output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(first_line(&out.stderr))
        }
    }

    pub fn status(&self) -> Option<Status> {
        let out = self
            .command(&[
                "status",
                "--porcelain=v1",
                "--branch",
                "-z",
                "--untracked-files=all",
            ])
            .output()
            .ok()
            .filter(|o| o.status.success())?;
        Some(parse_status(
            &self.root,
            &String::from_utf8_lossy(&out.stdout),
        ))
    }

    pub fn stage(&self, paths: &[PathBuf]) -> Result<(), String> {
        self.run_on(&["add", "-A"], paths).map(drop)
    }

    pub fn stage_all(&self) -> Result<(), String> {
        self.run(&["add", "-A"]).map(drop)
    }

    pub fn unstage(&self, paths: &[PathBuf]) -> Result<(), String> {
        if self
            .run(&["rev-parse", "--verify", "--quiet", "HEAD"])
            .is_ok()
        {
            self.run_on(&["restore", "--staged"], paths).map(drop)
        } else {
            // Nothing has been committed, so there is no HEAD to restore from.
            self.run_on(&["rm", "--cached", "-r", "--quiet"], paths)
                .map(drop)
        }
    }

    pub fn unstage_all(&self) -> Result<(), String> {
        self.unstage(std::slice::from_ref(&self.root))
    }

    /// Throws away every change under `path`: staged and not, and new files too.
    pub fn discard(&self, status: &Status, path: &Path) -> Result<(), String> {
        let under = |e: &(&PathBuf, &Entry)| e.0.starts_with(path);
        let (mut untracked, mut added, mut tracked) = (Vec::new(), Vec::new(), Vec::new());
        for (file, entry) in status.files.iter().filter(under) {
            match (entry.index, entry.worktree) {
                ('?', _) => untracked.push(file.clone()),
                ('A', _) => added.push(file.clone()),
                _ => tracked.push(file.clone()),
            }
        }
        if !tracked.is_empty() {
            self.run_on(
                &["restore", "--source=HEAD", "--staged", "--worktree"],
                &tracked,
            )?;
        }
        if !added.is_empty() {
            self.run_on(&["rm", "-f", "--quiet"], &added)?;
        }
        if !untracked.is_empty() {
            self.run_on(&["clean", "-f", "--quiet"], &untracked)?;
        }
        Ok(())
    }

    /// Commits what is staged. Gives git's summary line, like `[main abc123] message`.
    pub fn commit(&self, message: &str) -> Result<String, String> {
        if message.trim().is_empty() {
            return Err("Aborting commit due to empty message".into());
        }
        let said = self.run(&["commit", "-m", message])?;
        Ok(said.lines().next().unwrap_or("Committed").to_string())
    }

    /// The text of `file` at the last commit. `None` if it isn't in it.
    pub fn head_text(&self, file: &Path) -> Option<String> {
        let rel = file.strip_prefix(&self.root).ok()?;
        let spec = format!("HEAD:{}", rel.to_string_lossy().replace('\\', "/"));
        self.run(&["show", &spec]).ok()
    }

    /// Runs git in another thread, for commands that can take a while like `push`. The receiver
    /// gets the first line of what it said, or of its complaint, once it is done.
    pub fn spawn(&self, args: &[&str]) -> Receiver<Result<String, String>> {
        let (send, receive) = mpsc::channel();
        let mut command = self.command(args);
        std::thread::spawn(move || {
            let result = match command.output() {
                Ok(out) if out.status.success() => {
                    // Progress goes to stderr even when it works.
                    let said = if out.stdout.is_empty() {
                        &out.stderr
                    } else {
                        &out.stdout
                    };
                    Ok(first_line(said))
                }
                Ok(out) => Err(first_line(&out.stderr)),
                Err(e) => Err(e.to_string()),
            };
            let _ = send.send(result);
        });
        receive
    }
}

/// Reads the output of `git status --porcelain=v1 --branch -z`, whose paths are from `root`.
fn parse_status(root: &Path, text: &str) -> Status {
    let mut status = Status::default();
    let mut fields = text.split('\0').filter(|f| !f.is_empty());
    while let Some(field) = fields.next() {
        if let Some(head) = field.strip_prefix("## ") {
            let (name, tracking) = head.split_once(' ').unwrap_or((head, ""));
            status.branch = match head.strip_prefix("No commits yet on ") {
                Some(branch) => branch.to_string(),
                None if head.starts_with("HEAD (") => "HEAD".into(),
                None => name.split("...").next().unwrap_or(name).to_string(),
            };
            let number = |word: &str| {
                tracking
                    .split(word)
                    .nth(1)
                    .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0)
            };
            (status.ahead, status.behind) = (number("ahead "), number("behind "));
            continue;
        }
        let mut chars = field.chars();
        let (Some(index), Some(worktree)) = (chars.next(), chars.next()) else {
            continue;
        };
        let path = root.join(&field[3..]);
        // A rename or copy is followed by the name it had.
        if matches!(index, 'R' | 'C') || matches!(worktree, 'R' | 'C') {
            fields.next();
        }
        for dir in path.ancestors().skip(1).take_while(|d| *d != root) {
            let entry = Entry { index, worktree };
            let tone = status.dirs.entry(dir.to_path_buf()).or_insert(entry.tone());
            *tone = (*tone).max(entry.tone());
        }
        status.files.insert(path, Entry { index, worktree });
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_status_branch_changes_and_folders() {
        let root = Path::new("/r");
        let text = "## main...origin/main [ahead 2, behind 1]\0 M a.dj\0A  sub/b.dj\0?? sub/deep/c.dj\0R  new.dj\0old.dj\0UU x.dj\0";
        let status = parse_status(root, text);
        assert_eq!(
            (status.branch.as_str(), status.ahead, status.behind),
            ("main", 2, 1)
        );
        let tone = |p: &str| status.tone(Path::new(p));
        assert_eq!(tone("/r/a.dj"), Some(Tone::Modified));
        assert_eq!(tone("/r/sub/b.dj"), Some(Tone::Added));
        assert_eq!(tone("/r/sub/deep/c.dj"), Some(Tone::Untracked));
        assert_eq!(tone("/r/new.dj"), Some(Tone::Added));
        assert_eq!(tone("/r/x.dj"), Some(Tone::Conflict));
        // The name a rename had is not a file of its own, and folders take the strongest.
        assert_eq!(tone("/r/old.dj"), None);
        assert_eq!(tone("/r/sub"), Some(Tone::Added));
        assert_eq!(tone("/r/sub/deep"), Some(Tone::Untracked));
        assert_eq!(tone("/r"), None);
        assert_eq!(status.staged(), 2);
        let letters: Vec<char> = ["/r/a.dj", "/r/sub/b.dj", "/r/sub/deep/c.dj", "/r/x.dj"]
            .iter()
            .map(|p| status.files[Path::new(p)].letter())
            .collect();
        assert_eq!(letters, ['M', 'A', 'U', 'C']);
        // Other headings for the branch line.
        for (line, want) in [
            ("## No commits yet on trunk\0", "trunk"),
            ("## HEAD (no branch)\0", "HEAD"),
            ("## solo\0", "solo"),
        ] {
            assert_eq!(parse_status(root, line).branch, want);
        }
    }

    /// A repository in a fresh folder, or `None` where git isn't installed.
    fn repo(name: &str) -> Option<(Repo, PathBuf)> {
        let dir = std::env::temp_dir().join(format!("bwa_git_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        let ok = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["init", "-q", "-b", "main"])
            .status()
            .ok()?
            .success();
        ok.then(|| {
            for (key, value) in [("user.name", "T"), ("user.email", "t@example.com")] {
                Command::new("git")
                    .arg("-C")
                    .arg(&dir)
                    .args(["config", key, value])
                    .status()
                    .unwrap();
            }
            (Repo::discover(&dir).unwrap(), dir)
        })
    }

    #[test]
    fn stage_commit_edit_and_discard_in_a_real_repository() {
        let Some((repo, dir)) = repo("flow") else {
            return;
        };
        let file = repo.root.join("a.dj");
        std::fs::write(&file, "one\n").unwrap();
        let status = repo.status().unwrap();
        assert_eq!(status.branch, "main");
        assert_eq!(status.files[&file].letter(), 'U');
        // Stage, and commit (with an empty message refused).
        repo.stage(std::slice::from_ref(&file)).unwrap();
        assert_eq!(repo.status().unwrap().files[&file].letter(), 'A');
        assert!(repo.commit("  ").is_err());
        assert!(repo.commit("first").unwrap().contains("first"));
        assert!(repo.status().unwrap().files.is_empty());
        assert_eq!(repo.head_text(&file).as_deref(), Some("one\n"));
        // An edit shows, stages and unstages, and discarding restores the committed text.
        std::fs::write(&file, "two\n").unwrap();
        let status = repo.status().unwrap();
        assert_eq!(status.files[&file].letter(), 'M');
        repo.stage(std::slice::from_ref(&file)).unwrap();
        assert!(repo.status().unwrap().files[&file].staged());
        repo.unstage(std::slice::from_ref(&file)).unwrap();
        assert!(!repo.status().unwrap().files[&file].staged());
        repo.discard(&repo.status().unwrap(), &file).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\n");
        // A new file is deleted by discarding, folders included.
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/n.dj"), "n").unwrap();
        repo.discard(&repo.status().unwrap(), &repo.root.join("sub"))
            .unwrap();
        assert!(!dir.join("sub/n.dj").exists());
        assert_eq!(repo.head_text(&repo.root.join("nope.dj")), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unstaging_works_before_the_first_commit_and_a_background_command_reports() {
        let Some((repo, dir)) = repo("first") else {
            return;
        };
        let file = repo.root.join("a.dj");
        std::fs::write(&file, "x").unwrap();
        repo.stage_all().unwrap();
        assert!(repo.status().unwrap().files[&file].staged());
        repo.unstage_all().unwrap();
        assert_eq!(repo.status().unwrap().files[&file].letter(), 'U');
        // A push with nowhere to push to comes back as a message, not a hang.
        let said = repo.spawn(&["push"]).recv().unwrap();
        assert!(said.is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
