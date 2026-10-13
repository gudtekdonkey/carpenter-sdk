//! The crate boundary between the Carpenter AMM and any hook built on it.
//!
//! Two halves, and this file is the second:
//! 1. `carpenter-types` carries no FLOOR seed, layout, id or constant — the `compile_fail`
//!    doctests in `src/lib.rs` (each spells one FLOOR name through this crate and must NOT compile).
//! 2. Every crate in this workspace links `carpenter-types` and `carpenter-clmm-math` ALONE — never
//!    `floor-types`, `floor-math` or `floor-quote` — so no FLOOR name reaches the AMM side by any
//!    path. That is checked here, as text over the manifests and sources, because a `cargo metadata`
//!    walk would need cargo at test time and the manifests are the thing a reviewer edits.
//!
//! Adding a `floor-` dependency to any manifest below, or a `use floor_types::…` to any of its
//! sources, fails exactly one assertion by name.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The AMM side: every crate of this workspace. `(manifest, source root)`.
const AMM_SIDE: &[(&str, &str)] = &[
    ("crates/carpenter-types/Cargo.toml", "crates/carpenter-types/src"),
    ("crates/carpenter-clmm-math/Cargo.toml", "crates/carpenter-clmm-math/src"),
    ("crates/carpenter-quote/Cargo.toml", "crates/carpenter-quote/src"),
];

/// The FLOOR crates, as manifests spell them, and the paths sources spell them.
const FLOOR_CRATES: &[&str] = &["floor-types", "floor-math", "floor-quote"];
const FLOOR_PATHS: &[&str] = &["floor_types::", "floor_math::", "floor_quote::"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let p = entry.expect("dir entry").path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(at) => &line[..at],
        None => line,
    }
}

fn manifest_dependency_lines(manifest: &str) -> Vec<String> {
    // every non-comment line of every `[dependencies]` / `[dev-dependencies]` / `[build-dependencies]`
    // table, and every `[dependencies.<name>]` header
    let mut out = Vec::new();
    let mut in_deps = false;
    for line in manifest.lines() {
        let code = line.split('#').next().unwrap_or("").trim();
        if code.starts_with('[') {
            in_deps = code.contains("dependencies");
            if in_deps && code.contains("dependencies.") {
                // `[dependencies.<name>]`: the name is on the header line
                out.push(code.to_string());
            }
            continue;
        }
        if in_deps && !code.is_empty() {
            out.push(code.to_string());
        }
    }
    out
}

#[test]
fn the_amm_crates_link_no_floor_crate() {
    let root = repo_root();
    for (manifest, _) in AMM_SIDE {
        let path = root.join(manifest);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{manifest}: {e}"));
        for dep in manifest_dependency_lines(&text) {
            for floor in FLOOR_CRATES {
                assert!(
                    !dep.contains(floor),
                    "{manifest} links `{floor}` (`{dep}`): the AMM side takes `carpenter-types` alone"
                );
            }
        }
    }
}

#[test]
fn the_amm_sources_spell_no_floor_path() {
    let root = repo_root();
    for (_, src) in AMM_SIDE {
        let mut files = Vec::new();
        rust_files(&root.join(src), &mut files);
        assert!(!files.is_empty(), "no .rs files under {src}");
        for f in files {
            let text = fs::read_to_string(&f).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
            for (i, line) in text.lines().enumerate() {
                let code = strip_comment(line);
                for p in FLOOR_PATHS {
                    assert!(
                        !code.contains(p),
                        "{}:{}: spells `{p}` — the AMM side reaches its bytes through `carpenter_types::` only",
                        f.display(),
                        i + 1
                    );
                }
            }
        }
    }
}

/// The names the `compile_fail` doctests in `src/lib.rs` refuse are defined nowhere in this crate
/// (their definitions live in `floor-types`, whose own `tests/boundary.rs` checks that side).
#[test]
fn no_floor_name_is_defined_or_re_exported_here() {
    let here = repo_root().join("crates/carpenter-types/src");
    for file in ["seeds.rs", "layout.rs", "constants.rs", "ids.rs", "disc.rs", "hook.rs", "keys.rs", "pod.rs", "lib.rs"] {
        let text = fs::read_to_string(here.join(file)).unwrap();
        for needle in [
            "pub const FLOOR_POOL",
            "pub struct FloorPool",
            "pub struct Launch",
            "pub const SUPPLY:",
            "pub const FLOOR_HOOK_FLAGS",
            "pub const FLOOR_HOOK_ID",
            "pub use floor_types",
        ] {
            assert!(!text.contains(needle), "crates/carpenter-types/src/{file} defines or re-exports `{needle}`");
        }
    }
}
