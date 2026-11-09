//! End-to-end install tests: Workshop reuse, overlay, copy and verification.

use std::fs;
use std::path::{Path, PathBuf};

use nimby_core::formats::{generate_from_dir, ModManifest};
use nimby_core::install::{install_mod, InstallOptions};
use nimby_core::linker::{self, LinkMode};
use nimby_core::verify::verify_dir;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        Self { _dir: dir, root }
    }

    /// A mod source tree: `repo` is what the repository ships, `workshop` is
    /// what Steam downloaded. They can differ.
    fn make_mod(&self, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let dir = self.root.join(name);
        fs::create_dir_all(&dir).unwrap();
        for (rel, data) in files {
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, data).unwrap();
        }
        dir
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

const FILES: &[(&str, &[u8])] = &[
    ("mod.cpp", b"name=\"demo\";\n"),
    ("meta.cpp", b"publishedid = 123;\n"),
    ("addons/a.pbo", b"not-a-pbo-really"),
    ("keys/demo.bikey", b"key-bytes"),
];

fn manifest_for(fixture: &Fixture, name: &str) -> ModManifest {
    let src = fixture.make_mod(&format!("src-{name}"), FILES);
    generate_from_dir(name, &src).unwrap()
}

#[test]
fn full_workshop_match_creates_dir_link() {
    let fixture = Fixture::new();
    let manifest = manifest_for(&fixture, "@demo");
    let workshop = fixture.make_mod("workshop-123", FILES);

    let target = fixture.path().join("install").join("@demo");
    let roots = vec![workshop.clone()];
    let report = install_mod(
        &manifest,
        &target,
        &InstallOptions {
            workshop_roots: &roots,
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(report.mode, "dir-link");
    assert_eq!(report.downloaded_files, 0);
    assert_eq!(report.reused_bytes, manifest.total_size());
    assert!(linker::is_link(&target), "mod folder should be a link");

    let verify = verify_dir(&manifest, &target).unwrap();
    assert!(verify.is_ok(), "{:?}", verify);

    // A second install of the same mod must be allowed: it is managed.
    let again = install_mod(
        &manifest,
        &target,
        &InstallOptions {
            workshop_roots: &roots,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(again.mode, "dir-link");

    linker::remove_path(&target).unwrap();
    assert!(workshop.is_dir(), "workshop item must never be deleted");
}

#[test]
fn partial_workshop_match_links_files_and_downloads_the_rest() {
    let fixture = Fixture::new();
    let manifest = manifest_for(&fixture, "@demo");

    // Workshop item is missing addons/a.pbo and has a different mod.cpp.
    let workshop = fixture.make_mod(
        "workshop-partial",
        &[
            ("mod.cpp", b"name=\"other\";\n"),
            ("meta.cpp", b"publishedid = 123;\n"),
            ("keys/demo.bikey", b"key-bytes"),
        ],
    );
    // Local mirror of the repository payload for the two missing files.
    let repo_root = fixture.path().join("repo");
    fixture.make_mod(
        "repo/@demo",
        &[
            ("mod.cpp", b"name=\"demo\";\n"),
            ("addons/a.pbo", b"not-a-pbo-really"),
        ],
    );

    let target = fixture.path().join("install").join("@demo");
    let roots = vec![workshop];
    let report = install_mod(
        &manifest,
        &target,
        &InstallOptions {
            workshop_roots: &roots,
            local_repo: Some(&repo_root),
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(report.mode, "overlay");
    assert_eq!(report.linked_files, 2, "meta.cpp and the key are reused");
    assert_eq!(
        report.copied_files, 2,
        "the rest comes from the local mirror"
    );
    assert_eq!(report.downloaded_files, 0);
    assert!(target.join("mod.srf").is_file());

    let verify = verify_dir(&manifest, &target).unwrap();
    assert!(verify.is_ok(), "{:?}", verify);
}

#[test]
fn copy_mode_never_links() {
    let fixture = Fixture::new();
    let manifest = manifest_for(&fixture, "@demo");
    let workshop = fixture.make_mod("workshop-copy", FILES);
    let target = fixture.path().join("install").join("@demo");
    let roots = vec![workshop];

    let report = install_mod(
        &manifest,
        &target,
        &InstallOptions {
            mode: LinkMode::Copy,
            workshop_roots: &roots,
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(report.mode, "copy");
    assert_eq!(report.linked_files, 0);
    assert_eq!(
        report.downloaded_files, 0,
        "copy reads from the Workshop cache"
    );
    assert!(!linker::is_link(&target));
    assert!(verify_dir(&manifest, &target).unwrap().is_ok());
}

#[test]
fn passthrough_leaves_no_files_behind() {
    let fixture = Fixture::new();
    let manifest = manifest_for(&fixture, "@demo");
    let workshop = fixture.make_mod("workshop-pass", FILES);
    let target = fixture.path().join("install").join("@demo");
    let roots = vec![workshop.clone()];

    let report = install_mod(
        &manifest,
        &target,
        &InstallOptions {
            mode: LinkMode::Passthrough,
            workshop_roots: &roots,
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(report.mode, "passthrough");
    assert_eq!(report.target, workshop.display().to_string());
    assert!(
        !linker::exists(&target),
        "passthrough must not create the folder"
    );
}

#[test]
fn refuses_to_overwrite_unmanaged_directory() {
    let fixture = Fixture::new();
    let manifest = manifest_for(&fixture, "@demo");
    let target = fixture.path().join("install").join("@demo");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("user-file.txt"), b"keep me").unwrap();

    let err = install_mod(&manifest, &target, &InstallOptions::default()).unwrap_err();
    assert!(
        err.to_string().contains("not managed"),
        "unexpected error: {err}"
    );
    assert!(target.join("user-file.txt").is_file());

    // --force replaces it.
    let report = install_mod(
        &manifest,
        &target,
        &InstallOptions {
            force: true,
            local_repo: None,
            ..Default::default()
        },
    );
    assert!(report.is_err(), "no source configured, so it must fail");
    assert!(
        !target.join("user-file.txt").exists(),
        "force removes the old tree"
    );
}

#[test]
fn fails_when_a_file_has_no_source() {
    let fixture = Fixture::new();
    let manifest = manifest_for(&fixture, "@demo");
    let target = fixture.path().join("install").join("@demo");
    let err = install_mod(&manifest, &target, &InstallOptions::default()).unwrap_err();
    assert!(err.to_string().contains("missing file"), "got: {err}");
}
