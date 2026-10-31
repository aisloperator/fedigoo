//! `--install` / `--uninstall`: per-user installation of the executable, the
//! desktop entry and the icon, so FediGoo appears in application menus and
//! gets its icon on Wayland docks.  Nothing here touches the config dir.

use crate::APP_ID;
use gtk::glib;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const DESKTOP_TEMPLATE: &str = include_str!("../data/com.aisloperator.FediGoo.desktop.in");
const ICON_SVG: &[u8] = include_bytes!("../data/icons/com.aisloperator.FediGoo.svg");

struct Paths {
    bin: PathBuf,
    desktop: PathBuf,
    icon: PathBuf,
}

fn paths() -> Paths {
    let data = glib::user_data_dir();
    Paths {
        bin: glib::home_dir().join(".local/bin/fedigoo"),
        desktop: data.join(format!("applications/{APP_ID}.desktop")),
        icon: data.join(format!("icons/hicolor/scalable/apps/{APP_ID}.svg")),
    }
}

/// Quotes a path for a desktop entry's `Exec` key when needed.
fn exec_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    if !s.contains(|c: char| c.is_whitespace() || "\"'\\`$;&|<>()*?#~".contains(c)) {
        return s.into_owned();
    }
    let mut q = String::from("\"");
    for c in s.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            // The desktop-file string escape doubles the backslash.
            q.push_str("\\\\");
        }
        q.push(c);
    }
    q.push('"');
    q
}

/// Writes `bytes` to `path` via a temporary file and rename, so an existing
/// file (even a running executable) is replaced atomically.
fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("fedigoo-tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
    std::fs::rename(&tmp, path)
}

fn on_path(dir: &Path) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == dir))
}

pub fn install() -> io::Result<()> {
    let p = paths();
    let exe = std::env::current_exe()?;
    if exe.canonicalize().ok() != p.bin.canonicalize().ok() {
        write_atomic(&p.bin, &std::fs::read(&exe)?, 0o755)?;
        println!("Installed {}", p.bin.display());
    } else {
        println!("Already running from {}", p.bin.display());
    }
    write_atomic(&p.icon, ICON_SVG, 0o644)?;
    println!("Installed {}", p.icon.display());
    let desktop = DESKTOP_TEMPLATE.replace("@EXEC@", &exec_quote(&p.bin));
    write_atomic(&p.desktop, desktop.as_bytes(), 0o644)?;
    println!("Installed {}", p.desktop.display());
    if let Some(dir) = p.bin.parent()
        && !on_path(dir)
    {
        println!("Note: {} is not on your PATH.", dir.display());
    }
    println!(
        "FediGoo should now appear in your application menu (you may need to log out and back in)."
    );
    Ok(())
}

pub fn uninstall() -> io::Result<()> {
    let p = paths();
    for path in [&p.desktop, &p.icon, &p.bin] {
        match std::fs::remove_file(path) {
            Ok(()) => println!("Removed {}", path.display()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    println!(
        "Your accounts and settings in {} were left in place.",
        crate::config::dir().display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_quoting() {
        assert_eq!(
            exec_quote(Path::new("/home/a/.local/bin/fedigoo")),
            "/home/a/.local/bin/fedigoo"
        );
        assert_eq!(
            exec_quote(Path::new("/home/a b/fedigoo")),
            "\"/home/a b/fedigoo\""
        );
        assert_eq!(exec_quote(Path::new("/x/$y")), "\"/x/\\\\$y\"");
    }
}
