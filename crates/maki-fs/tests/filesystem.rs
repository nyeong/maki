use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use maki_core::{Error, ProjectDiagnosticKind};
use maki_fs::{
    find_project_root, list_maki_files, load_project, load_project_config,
    load_project_with_config, read_maki_sources, render_file_html,
};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(name: &str) -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("maki-fs-{name}-{}-{sequence}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn write(&self, relative: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn adapter_loads_manifest_sources_and_renders_a_project_file() {
    let project = TestDirectory::new("load-project");
    project.write(
        "maki.toml",
        "[project]\ntitle = \"Filesystem fixture\"\nsource = \"docs\"\n",
    );
    let page = project.write("docs/index.maki", "--^ title: Home\n\nSee [[child]].");
    project.write("docs/child.maki", "--^ title: Child\n");

    let config = load_project_config(&project.path).unwrap();
    let source_root = config.project_source_root(&project.path);
    let maki = load_project_with_config(&source_root, config).unwrap();

    assert_eq!(maki.notes_len(), 2);
    assert_eq!(maki.config().project_title(), Some("Filesystem fixture"));
    assert!(maki.diagnostics().is_empty());
    assert!(render_file_html(&maki, &page).unwrap().contains("Home"));
    assert!(maki.snapshot_compile_duration() > std::time::Duration::ZERO);
}

#[test]
fn adapter_finds_the_nearest_project_manifest_from_a_file() {
    let project = TestDirectory::new("find-root");
    project.write("maki.toml", "[project]\n");
    let page = project.write("docs/nested/page.maki", "Page");

    assert_eq!(
        find_project_root(&page).unwrap(),
        Some(fs::canonicalize(&project.path).unwrap())
    );
}

#[cfg(unix)]
#[test]
fn discovery_is_sorted_and_does_not_follow_excluded_directories() {
    use std::os::unix::fs::symlink;

    let project = TestDirectory::new("discovery");
    project.write("z.maki", "z");
    project.write("notes/a.maki", "a");
    project.write("notes/not-maki.txt", "text");
    project.write(".hidden/hidden.maki", "hidden");
    project.write("node_modules/package/generated.maki", "generated");
    project.write("target/generated.maki", "generated");
    project.write("linked-target/inside.maki", "inside");
    symlink(
        project.path.join("linked-target"),
        project.path.join("directory-link"),
    )
    .unwrap();
    symlink(
        project.path.join("notes/a.maki"),
        project.path.join("linked-file.maki"),
    )
    .unwrap();

    assert_eq!(
        list_maki_files(&project.path).unwrap(),
        vec![
            PathBuf::from("linked-file.maki"),
            PathBuf::from("linked-target/inside.maki"),
            PathBuf::from("notes/a.maki"),
            PathBuf::from("z.maki"),
        ]
    );
}

#[test]
fn project_load_preserves_a_read_failure_as_input_data() {
    let project = TestDirectory::new("read-failure");
    project.write("invalid.maki", [0xff, 0xfe]);

    let maki = load_project(&project.path).unwrap();

    assert!(matches!(
        maki.analysis(),
        Err(Error::ReadNoteFailed(path)) if path.ends_with(Path::new("invalid.maki"))
    ));
    assert!(
        maki.diagnostics()
            .iter()
            .any(|diagnostic| matches!(diagnostic.kind(), ProjectDiagnosticKind::ReadFailed))
    );
    assert!(matches!(
        read_maki_sources(&project.path),
        Err(Error::ReadNoteFailed(path)) if path.ends_with(Path::new("invalid.maki"))
    ));
}

#[test]
fn adapter_rejects_missing_and_non_directory_roots() {
    let project = TestDirectory::new("invalid-root");
    let file = project.write("single.maki", "Single");
    let missing = project.path.join("missing");

    assert!(matches!(
        load_project(&file),
        Err(Error::RootNotDirectory(path)) if path == file
    ));
    assert!(matches!(
        load_project(&missing),
        Err(Error::RootNotFound(path)) if path == missing
    ));
}
