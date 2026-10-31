//! Compiles `data/` into a GResource bundle that is embedded in the binary,
//! using `glib-compile-resources` (from the GLib dev tools that GTK's dev
//! package already requires) rather than an extra crate.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("fedigoo.gresource");
    println!("cargo:rerun-if-changed=data");
    let status = Command::new("glib-compile-resources")
        .arg("--sourcedir=data")
        .arg(format!("--target={}", out.display()))
        .arg("data/fedigoo.gresource.xml")
        .status()
        .expect("could not run glib-compile-resources (Debian package libgio-2.0-dev-bin)");
    assert!(status.success(), "glib-compile-resources failed");
}
