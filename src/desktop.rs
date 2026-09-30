//! Puts the app in the desktop's menus with its icon, on Linux. Wayland has no window icon of its
//! own: the compositor finds the icon from the window's app id, through a `.desktop` file of the
//! same name, so without one the window gets a plain stand-in.

use std::io;
use std::path::{Path, PathBuf};

/// The app id the window announces, and the name of the `.desktop` file and the icon.
pub const APP_ID: &str = "vim-book-writer";

const ICONS: [(u32, &[u8]); 4] = [
    (32, include_bytes!("../packaging/icons/32x32.png")),
    (128, include_bytes!("../packaging/icons/128x128.png")),
    (256, include_bytes!("../packaging/icons/256x256.png")),
    (512, include_bytes!("../packaging/icons/512x512.png")),
];

/// The user's data folder: `$XDG_DATA_HOME`, or `~/.local/share`.
pub fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".local/share")))
}

/// The text of the `.desktop` file for the program at `exe`.
fn entry(exe: &Path) -> String {
    // The Exec line quotes the path, and escapes what the quotes don't cover.
    let path: String = exe
        .to_string_lossy()
        .chars()
        .flat_map(|c| match c {
            '"' | '`' | '$' | '\\' => vec!['\\', c],
            c => vec![c],
        })
        .collect();
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Vim Book Writer\n\
         Comment=A fast, lightweight word processor for djot, with vim controls\n\
         Exec=\"{path}\" %F\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         Categories=Office;TextEditor;\n\
         MimeType=text/x-djot;\n\
         StartupWMClass={APP_ID}\n"
    )
}

/// Writes `bytes` to `path` unless it already holds them, making its folder if needed.
fn put(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if std::fs::read(path).is_ok_and(|now| now == bytes) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, bytes)
}

/// Adds the menu entry for the program at `exe` and its icons under `data`. It writes only what
/// is missing or out of date, so it is cheap to call at every start.
pub fn install(data: &Path, exe: &Path) -> io::Result<()> {
    put(
        &data.join("applications").join(format!("{APP_ID}.desktop")),
        entry(exe).as_bytes(),
    )?;
    for (size, png) in ICONS {
        let path = data
            .join(format!("icons/hicolor/{size}x{size}/apps"))
            .join(format!("{APP_ID}.png"));
        put(&path, png)?;
    }
    Ok(())
}

/// `install` for the user running the program: their data folder and this program.
pub fn install_for_user() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    match data_home() {
        Some(data) => install(&data, &exe),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_the_entry_and_icons_once_and_follows_the_program_when_it_moves() {
        let dir = std::env::temp_dir().join("bwa_desktop");
        let _ = std::fs::remove_dir_all(&dir);
        install(&dir, Path::new("/opt/my apps/vim-book-writer")).unwrap();
        let file = dir.join("applications/vim-book-writer.desktop");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("Exec=\"/opt/my apps/vim-book-writer\" %F"));
        assert!(
            text.contains("Icon=vim-book-writer")
                && text.contains("StartupWMClass=vim-book-writer")
        );
        for size in [32, 128, 256, 512] {
            let icon = dir.join(format!(
                "icons/hicolor/{size}x{size}/apps/vim-book-writer.png"
            ));
            assert!(std::fs::read(icon).unwrap().starts_with(b"\x89PNG"));
        }
        // Nothing is written again when nothing changed, and a moved program is followed.
        let stamp = std::fs::metadata(&file).unwrap().modified().unwrap();
        install(&dir, Path::new("/opt/my apps/vim-book-writer")).unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), stamp);
        install(&dir, Path::new("/usr/bin/vim-book-writer")).unwrap();
        assert!(
            std::fs::read_to_string(&file)
                .unwrap()
                .contains("Exec=\"/usr/bin/vim-book-writer\"")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
