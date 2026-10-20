//! The launcher must define the exception personality under every name the
//! toolchain's `core` asks for.
//!
//! `core` is precompiled, and its unwind tables name the personality routine
//! the linker has to find. Rust moved that routine to the def path
//! `__rustc::rust_eh_personality`, and the v0 mangling of an internal def
//! path hashes the compiler version, so every compiler version asks for a
//! name of its own. The launcher cannot spell the name out: it would bind
//! the launcher to one toolchain and fail against the next. The build script
//! reads the name out of the toolchain it builds the launcher with.
//!
//! The link covers the name of the toolchain the tests run with: the crate's
//! own build compiles the launcher, and a `core` that asks for a name the
//! launcher does not define fails the link. No toolchain in this repository
//! asks for a second name, so this test reads the name out of the toolchain's
//! `core` on its own, and a second test hands the helper names of the shape a
//! newer `core` asks for, with disambiguators no toolchain here spells.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "../launcher/personality.rs"]
mod personality;

/// The plain name of the personality. The mangled name a newer `core` asks
/// for ends in it, with the length of the name in front of it.
const PLAIN: &[u8] = b"rust_eh_personality";

/// The toolchain the test builds the launcher with.
///
/// `KACHE_LAUNCHER_RUSTC` names the toolchain to use, so the test can run
/// against one other than the toolchain the tests are built with: the name
/// the launcher needs depends on the compiler version, and only a toolchain
/// built after the move to the `__rustc` def path asks for a second name.
fn toolchain() -> PathBuf {
    std::env::var_os("KACHE_LAUNCHER_RUSTC")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("RUSTC"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("rustc"))
}

/// The `core` image of the given toolchain.
fn core_image(rustc: &Path) -> Vec<u8> {
    let image = personality::core_image(rustc, None).expect("the toolchain has a precompiled core");
    std::fs::read(image).unwrap()
}

/// The personality names the given `core` image asks for.
///
/// The image is split on the NUL bytes the compiler writes after every name
/// in the unwind tables, and the names behind the prefix it writes in front
/// of them are kept. The helper the build script uses reads the same names
/// with a different scan; a helper that answers wrongly must not be able to
/// agree with itself.
fn asked_names(image: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    for field in image.split(|byte| *byte == 0) {
        let Some(name) = field.strip_prefix(b"DW.ref.") else {
            continue;
        };
        if !name.ends_with(PLAIN) {
            continue;
        }
        let name = String::from_utf8_lossy(name).into_owned();
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Compile the launcher the way the build script does, with the given
/// personality exports.
fn compile(rustc: &Path, exports: &str, out: &Path) -> Output {
    let mut command = Command::new(rustc);
    command
        .args(["--edition", "2024", "--crate-type", "bin"])
        .args(["--crate-name", "kache_build_script_launcher"])
        .args([
            "-C",
            "opt-level=s",
            "-C",
            "panic=abort",
            "-C",
            "debuginfo=0",
        ])
        .arg("-o")
        .arg(out);
    if !exports.is_empty() {
        let file = out.with_extension("personality.rs");
        std::fs::write(&file, exports).unwrap();
        command
            .env("KACHE_LAUNCHER_PERSONALITY_FILE", &file)
            .arg("--cfg")
            .arg("kache_personality_export")
            .arg("--check-cfg")
            .arg("cfg(kache_personality_export)");
    }
    // The launcher's own `link(name = "c")` puts libc before the rlibs, which
    // nixpkgs' stack-protectored compiler_builtins needs repeated. See build.rs.
    if cfg!(all(target_os = "linux", target_env = "gnu")) {
        command.args(["-C", "link-arg=-lc"]);
    }
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("launcher/build_script.rs");
    command
        .arg(source)
        .output()
        .expect("running rustc on the launcher")
}

/// Whether the image holds the given bytes.
fn contains(image: &[u8], needle: &[u8]) -> bool {
    image.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn the_launcher_defines_the_names_the_toolchains_core_asks_for() {
    let rustc = toolchain();
    let core = core_image(&rustc);
    let asked = asked_names(&core);
    assert!(!asked.is_empty(), "the core image names no personality");
    // The helper the build script uses must find the same names.
    assert_eq!(personality::names_in(&core), asked);

    let fixture = tempfile::tempdir().unwrap();
    let out = fixture.path().join("launcher");
    let exports = personality::source(&asked);
    let output = compile(&rustc, &exports, &out);
    assert!(output.status.success(), "{output:?}");

    // The name reaches the binary as a symbol-table entry. No ELF parser is a
    // dependency of this crate, and a name this specific appears nowhere else
    // in the image.
    let launcher = std::fs::read(&out).unwrap();
    for name in &asked {
        if name == personality::LEGACY {
            // The plain name is the launcher's own definition, and the
            // crate's own link covers it: the pinned toolchain's `core` asks
            // for it, so a launcher without it does not link.
            continue;
        }
        assert!(
            contains(&launcher, name.as_bytes()),
            "the launcher defines no `{name}`, which this toolchain's core asks for"
        );
    }
}

#[test]
fn a_name_no_toolchain_here_spells_is_derived_and_defined() {
    // The v0 mangling of the `__rustc::rust_eh_personality` def path ends in
    // the plain name, and the disambiguator in front of it hashes the
    // compiler version, so two compiler versions ask for two names of their
    // own. These two have the shape of such a name, with a disambiguator no
    // toolchain in this repository spells.
    let spelled = [
        "_RNvCsAAAAAAAAAAAAAAAA_7___rustc19rust_eh_personality",
        "_RNvCsBBBBBBBBBBBBBBBB_7___rustc19rust_eh_personality",
    ];
    let mut image = Vec::new();
    for name in spelled {
        image.extend_from_slice(b"DW.ref.");
        image.extend_from_slice(name.as_bytes());
        image.push(0);
    }

    // The names are read out of the image, not written down beside it.
    assert_eq!(asked_names(&image), spelled);
    assert_eq!(personality::names_in(&image), spelled);

    // The launcher links against the toolchain's own `core`, so it needs that
    // toolchain's name as well.
    let rustc = toolchain();
    let mut names = personality::names_in(&core_image(&rustc));
    names.extend(spelled.iter().map(|name| name.to_string()));

    let fixture = tempfile::tempdir().unwrap();
    let out = fixture.path().join("launcher");
    let output = compile(&rustc, &personality::source(&names), &out);
    assert!(output.status.success(), "{output:?}");

    let launcher = std::fs::read(&out).unwrap();
    for name in spelled {
        assert!(
            contains(&launcher, name.as_bytes()),
            "the launcher defines no `{name}`"
        );
    }
}
