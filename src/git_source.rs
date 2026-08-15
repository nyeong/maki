use std::{
    ffi::{OsStr, OsString},
    fmt::Display,
    fs,
    hash::Hasher,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    maki::{self, Maki, MakiConfig, MakiConfigOverrides},
    metrics::Metrics,
    web,
};

const DEFAULT_FETCH_INTERVAL: Duration = Duration::from_secs(60);
const REPOSITORY_GIT_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_PREFIX",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
];

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GitServeConfig {
    pub(crate) url: String,
    pub(crate) branch: String,
    pub(crate) state_dir: PathBuf,
    pub(crate) fetch_interval: Duration,
}

impl GitServeConfig {
    pub(crate) fn new(url: String) -> Self {
        Self {
            url,
            branch: "main".to_string(),
            state_dir: default_state_dir(),
            fetch_interval: DEFAULT_FETCH_INTERVAL,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GitSource {
    config: GitServeConfig,
    site_dir: PathBuf,
    mirror_dir: PathBuf,
    releases_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GitCheckout {
    commit: String,
    root: PathBuf,
}

impl GitCheckout {
    pub(crate) fn commit(&self) -> &str {
        &self.commit
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
}

#[derive(Debug)]
pub(crate) enum Error {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    GitCommandFailed {
        args: Vec<String>,
        status: Option<i32>,
        stderr: String,
    },
    MissingProjectFile {
        root: PathBuf,
    },
    SymlinkUnsupported {
        path: PathBuf,
    },
    Maki {
        source: maki::Error,
    },
}

impl From<maki::Error> for Error {
    fn from(source: maki::Error) -> Self {
        Self::Maki { source }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io { path, source } => {
                write!(f, "git source IO error at {}: {}", path.display(), source)
            }
            Error::GitCommandFailed {
                args,
                status,
                stderr,
            } => {
                write!(
                    f,
                    "git command failed (status {:?}): git {}",
                    status,
                    args.join(" ")
                )?;
                if !stderr.trim().is_empty() {
                    write!(f, ": {}", stderr.trim())?;
                }
                Ok(())
            }
            Error::MissingProjectFile { root } => {
                write!(
                    f,
                    "git checkout is missing {} at {}",
                    maki::PROJECT_FILE_NAME,
                    root.display()
                )
            }
            Error::SymlinkUnsupported { path } => {
                write!(
                    f,
                    "git checkout contains unsupported symlink: {}",
                    path.display()
                )
            }
            Error::Maki { source } => write!(f, "{}", source),
        }
    }
}

pub(crate) fn default_state_dir() -> PathBuf {
    default_state_dir_for_os()
}

#[cfg(target_os = "linux")]
fn default_state_dir_for_os() -> PathBuf {
    PathBuf::from("/var/lib/maki")
}

#[cfg(target_os = "macos")]
fn default_state_dir_for_os() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("maki")
        })
        .unwrap_or_else(|| PathBuf::from("/Library/Application Support/maki"))
}

#[cfg(target_os = "windows")]
fn default_state_dir_for_os() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .map(|program_data| program_data.join("maki"))
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData\maki"))
}

#[cfg(all(
    not(target_os = "linux"),
    not(target_os = "macos"),
    not(target_os = "windows")
))]
fn default_state_dir_for_os() -> PathBuf {
    PathBuf::from("/var/lib/maki")
}

pub(crate) fn parse_fetch_interval(raw: &str) -> Result<Duration, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("empty duration".to_string());
    }

    let (digits, unit) = raw
        .char_indices()
        .find(|(_, ch)| !ch.is_ascii_digit())
        .map_or((raw, ""), |(index, _)| raw.split_at(index));
    if digits.is_empty() {
        return Err(format!("invalid duration: {raw}"));
    }

    let value = digits
        .parse::<u64>()
        .map_err(|_| format!("invalid duration: {raw}"))?;

    match unit {
        "" | "s" => Ok(Duration::from_secs(value)),
        "m" => Ok(Duration::from_secs(value.saturating_mul(60))),
        "h" => Ok(Duration::from_secs(value.saturating_mul(60 * 60))),
        "ms" => Ok(Duration::from_millis(value)),
        _ => Err(format!("invalid duration unit: {unit}")),
    }
}

impl GitSource {
    pub(crate) fn new(config: GitServeConfig) -> Self {
        let site_id = site_id(&config.url, &config.branch);
        let site_dir = config.state_dir.join("sites").join(site_id);
        let mirror_dir = site_dir.join("repo.git");
        let releases_dir = site_dir.join("releases");

        Self {
            config,
            site_dir,
            mirror_dir,
            releases_dir,
        }
    }

    pub(crate) fn fetch_interval(&self) -> Duration {
        self.config.fetch_interval
    }

    pub(crate) fn prepare(&self) -> Result<GitCheckout, Error> {
        self.ensure_mirror()?;
        self.fetch()?;
        self.checkout_current_branch()
    }

    pub(crate) fn refresh(&self) -> Result<GitCheckout, Error> {
        self.fetch()?;
        self.checkout_current_branch()
    }

    pub(crate) fn load_maki(
        &self,
        checkout: &GitCheckout,
        config_overrides: &MakiConfigOverrides,
        metrics: &Metrics,
    ) -> Result<Maki, Error> {
        reject_symlinks(checkout.root())?;

        let project_file = checkout.root().join(maki::PROJECT_FILE_NAME);
        if !project_file.is_file() {
            return Err(Error::MissingProjectFile {
                root: checkout.root().to_path_buf(),
            });
        }

        let mut config = MakiConfig::load_project(checkout.root())?;
        config_overrides.apply_to(&mut config);
        let source_root = config.project_source_root(checkout.root());
        Ok(Maki::load_with_config_metered(
            &source_root,
            config,
            metrics,
        )?)
    }

    pub(crate) fn record_active(&self, checkout: &GitCheckout) -> Result<(), Error> {
        let content = format!(
            "branch = \"{}\"\nactive_commit = \"{}\"\nactive_path = \"{}\"\nupdated_at_unix = {}\n",
            escape_toml_string(&self.config.branch),
            checkout.commit(),
            escape_toml_string(&checkout.root().display().to_string()),
            unix_timestamp()
        );
        write_file(&self.site_dir.join("state.toml"), content.as_bytes())
    }

    pub(crate) fn record_failure(&self, message: &str) -> Result<(), Error> {
        let content = format!(
            "branch = \"{}\"\nlast_error = \"{}\"\nfailed_at_unix = {}\n",
            escape_toml_string(&self.config.branch),
            escape_toml_string(message),
            unix_timestamp()
        );
        write_file(&self.site_dir.join("last-error.toml"), content.as_bytes())
    }

    fn ensure_mirror(&self) -> Result<(), Error> {
        create_dir_all(&self.site_dir)?;

        if self.mirror_dir.is_dir() {
            git_run(args([
                os("-C"),
                self.mirror_dir.as_os_str(),
                os("remote"),
                os("set-url"),
                os("origin"),
                OsStr::new(&self.config.url),
            ]))?;
            return Ok(());
        }

        git_run(args([
            os("clone"),
            os("--mirror"),
            OsStr::new(&self.config.url),
            self.mirror_dir.as_os_str(),
        ]))
    }

    fn fetch(&self) -> Result<(), Error> {
        git_run(args([
            os("-C"),
            self.mirror_dir.as_os_str(),
            os("fetch"),
            os("--prune"),
            os("origin"),
        ]))
    }

    fn checkout_current_branch(&self) -> Result<GitCheckout, Error> {
        let commit = self.current_commit()?;
        let root = self.releases_dir.join(&commit);
        if root.is_dir() {
            return Ok(GitCheckout { commit, root });
        }

        create_dir_all(&self.releases_dir)?;
        let tmp = self
            .releases_dir
            .join(format!(".tmp-{}-{}", commit, std::process::id()));
        remove_dir_all_if_exists(&tmp)?;
        create_dir_all(&tmp)?;

        let checkout_result = git_run(args([
            os("--git-dir"),
            self.mirror_dir.as_os_str(),
            os("--work-tree"),
            tmp.as_os_str(),
            os("checkout"),
            os("--force"),
            OsStr::new(&commit),
            os("--"),
            os("."),
        ]));

        if let Err(error) = checkout_result {
            let _ = fs::remove_dir_all(&tmp);
            return Err(error);
        }

        fs::rename(&tmp, &root).map_err(|source| Error::Io {
            path: root.clone(),
            source,
        })?;
        Ok(GitCheckout { commit, root })
    }

    fn current_commit(&self) -> Result<String, Error> {
        let ref_name = format!("refs/heads/{}^{{commit}}", self.config.branch);
        let output = git_output(args([
            os("-C"),
            self.mirror_dir.as_os_str(),
            os("rev-parse"),
            OsStr::new(&ref_name),
        ]))?;
        Ok(output.trim().to_string())
    }
}

pub(crate) fn spawn_updater(
    source: GitSource,
    config_overrides: MakiConfigOverrides,
    state: Arc<web::AppState>,
    initial_commit: String,
) {
    thread::spawn(move || {
        let mut active_commit = initial_commit;

        loop {
            thread::sleep(source.fetch_interval());

            let refresh_started = Instant::now();
            let checkout = match source.refresh() {
                Ok(checkout) => {
                    state
                        .metrics()
                        .record_git_refresh("ok", refresh_started.elapsed());
                    checkout
                }
                Err(error) => {
                    state
                        .metrics()
                        .record_git_refresh("error", refresh_started.elapsed());
                    eprintln!("Failed to refresh git source: {}", error);
                    let _ = source.record_failure(&error.to_string());
                    continue;
                }
            };

            let reload_started = Instant::now();
            if checkout.commit() == active_commit {
                state
                    .metrics()
                    .record_project_reload("git", "unchanged", reload_started.elapsed());
                continue;
            }

            let maki = match source.load_maki(&checkout, &config_overrides, state.metrics()) {
                Ok(maki) => maki,
                Err(error) => {
                    state
                        .metrics()
                        .record_project_reload("git", "error", reload_started.elapsed());
                    eprintln!("Failed to load git checkout: {}", error);
                    let _ = source.record_failure(&error.to_string());
                    continue;
                }
            };

            if let Err(error) = state.replace_maki(maki) {
                state
                    .metrics()
                    .record_project_reload("git", "error", reload_started.elapsed());
                eprintln!("Failed to activate git checkout: {}", error);
                let _ = source.record_failure(&error.to_string());
                continue;
            }

            active_commit = checkout.commit().to_string();
            state
                .metrics()
                .record_project_reload("git", "updated", reload_started.elapsed());
            if let Err(error) = source.record_active(&checkout) {
                eprintln!("Failed to record active git checkout: {}", error);
            }
            println!("Activated git commit {}", checkout.commit());
        }
    });
}

fn reject_symlinks(root: &Path) -> Result<(), Error> {
    fn walk(path: &Path) -> Result<(), Error> {
        for entry in fs::read_dir(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })? {
            let entry = entry.map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })?;
            let entry_path = entry.path();
            let metadata = fs::symlink_metadata(&entry_path).map_err(|source| Error::Io {
                path: entry_path.clone(),
                source,
            })?;
            let file_type = metadata.file_type();

            if file_type.is_symlink() {
                return Err(Error::SymlinkUnsupported { path: entry_path });
            }

            if file_type.is_dir() {
                walk(&entry_path)?;
            }
        }

        Ok(())
    }

    walk(root)
}

fn site_id(url: &str, branch: &str) -> String {
    let slug = repo_slug(url);
    let mut hasher = Fnv1a64::default();
    hasher.write(url.as_bytes());
    hasher.write(&[0]);
    hasher.write(branch.as_bytes());
    format!("{slug}-{:016x}", hasher.finish())
}

fn repo_slug(url: &str) -> String {
    let candidate = url
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("repo")
        .trim_end_matches(".git");
    let slug = sanitize_path_component(candidate);
    if slug.is_empty() {
        "repo".to_string()
    } else {
        slug
    }
}

fn sanitize_path_component(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[derive(Default)]
struct Fnv1a64(u64);

impl Hasher for Fnv1a64 {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        if self.0 == 0 {
            self.0 = 0xcbf29ce484222325;
        }
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}

fn args<const N: usize>(items: [&OsStr; N]) -> Vec<OsString> {
    items.iter().map(|item| item.to_os_string()).collect()
}

fn os(value: &str) -> &OsStr {
    OsStr::new(value)
}

fn git_run(args: Vec<OsString>) -> Result<(), Error> {
    let _ = git_output(args)?;
    Ok(())
}

fn git_output(args: Vec<OsString>) -> Result<String, Error> {
    let mut command = Command::new("git");
    command.arg("-c").arg("core.fsmonitor=false").args(&args);
    for key in REPOSITORY_GIT_ENV {
        command.env_remove(key);
    }

    let output = command.output().map_err(|source| Error::Io {
        path: PathBuf::from("git"),
        source,
    })?;

    if !output.status.success() {
        return Err(Error::GitCommandFailed {
            args: args
                .iter()
                .map(|arg| arg.to_string_lossy().to_string())
                .collect(),
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn create_dir_all(path: &Path) -> Result<(), Error> {
    fs::create_dir_all(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn remove_dir_all_if_exists(path: &Path) -> Result<(), Error> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn write_file(path: &Path, content: &[u8]) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    fs::write(path, content).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn escape_toml_string(raw: &str) -> String {
    let mut escaped = String::new();
    for ch in raw.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fetch_interval_supports_common_units() {
        assert_eq!(
            parse_fetch_interval("250ms"),
            Ok(Duration::from_millis(250))
        );
        assert_eq!(parse_fetch_interval("30s"), Ok(Duration::from_secs(30)));
        assert_eq!(parse_fetch_interval("5m"), Ok(Duration::from_secs(300)));
        assert_eq!(parse_fetch_interval("2h"), Ok(Duration::from_secs(7200)));
        assert_eq!(parse_fetch_interval("42"), Ok(Duration::from_secs(42)));
    }

    #[test]
    fn site_id_keeps_url_out_of_state_path() {
        let id = site_id("https://token@example.com/nyeong/blog.git", "main");

        assert!(id.starts_with("blog-"));
        assert!(!id.contains("token"));
        assert!(!id.contains("example.com"));
    }
}
