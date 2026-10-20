//! The names a toolchain's `core` asks the exception personality under.
//!
//! `core` is precompiled, and its unwind tables name the personality routine
//! the linker has to find. Rust moved that routine to the def path
//! `__rustc::rust_eh_personality`, and the v0 mangling of an internal def
//! path hashes the compiler version, so every compiler version asks for a
//! name of its own. A launcher that spells the name out links against one
//! toolchain and fails against the next, so the name is read out of the
//! toolchain the launcher is built with instead.
//!
//! The build script and the tests share this file: `build.rs` reads it as a
//! module, and so does `tests/launcher_personality_test.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The name the personality had before Rust moved it to the `__rustc` def
/// path. A `core` built before the move asks for this name, so the launcher
/// defines it under that name for every toolchain.
pub const LEGACY: &str = "rust_eh_personality";

/// The prefix the compiler writes in front of the name of the personality in
/// the unwind tables of `core`.
const PREFIX: &[u8] = b"DW.ref.";

/// The `core` image of the given toolchain and target.
///
/// The target's own `core` is the one the launcher links against, so its
/// unwind tables are the ones that decide which name the launcher needs.
/// `None` for the target means the host.
pub fn core_image(rustc: &Path, target: Option<&str>) -> Option<PathBuf> {
    let mut command = Command::new(rustc);
    command.args(["--print", "target-libdir"]);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let directory = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
    let mut images: Vec<PathBuf> = std::fs::read_dir(directory)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("libcore-") && name.ends_with(".rlib"))
        })
        .collect();
    images.sort();
    images.into_iter().next()
}

/// The personality names the given `core` image asks for.
///
/// The name the launcher already defines is among them on a toolchain built
/// before the move to the `__rustc` def path; a toolchain built after it asks
/// for the mangled name instead.
pub fn names_in(image: &[u8]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for start in occurrences(image, PREFIX) {
        let rest = &image[start + PREFIX.len()..];
        // The name is a NUL-terminated string in the unwind table.
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(rest.len());
        let name = &rest[..end];
        // The mangled name ends in the plain name with its length in front
        // of it, and the plain name is the one the launcher defines already.
        if !name.ends_with(LEGACY.as_bytes()) || !is_symbol(name) {
            continue;
        }
        let name = String::from_utf8_lossy(name).into_owned();
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// The Rust source that defines the personality under each of the given
/// names.
///
/// The plain name is left out: the launcher defines it itself, for the
/// toolchains built before the move to the `__rustc` def path.
pub fn source(names: &[String]) -> String {
    let mut source = String::new();
    for (index, name) in names
        .iter()
        .filter(|name| name.as_str() != LEGACY)
        .enumerate()
    {
        source.push_str(&format!(
            "/// The personality under the name this toolchain's `core` asks for.\n\
             #[unsafe(export_name = \"{name}\")]\n\
             extern \"C\" fn kache_personality_{index}() {{}}\n\n"
        ));
    }
    source
}

/// The offsets of every occurrence of `needle` in `haystack`.
fn occurrences(haystack: &[u8], needle: &[u8]) -> impl Iterator<Item = usize> {
    haystack
        .windows(needle.len())
        .enumerate()
        .filter_map(move |(index, window)| (window == needle).then_some(index))
}

/// Whether the bytes spell a symbol name.
fn is_symbol(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'$'))
}
