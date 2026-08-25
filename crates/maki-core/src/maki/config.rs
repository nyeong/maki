use std::path::{Component, Path, PathBuf};

use super::{Error, PROJECT_FILE_NAME};

#[derive(Debug, PartialEq, Clone)]
pub struct MakiConfig {
    project_title: Option<String>,
    source_dir: PathBuf,
    home_mode: HomeMode,
    publish_policy: PublishPolicy,
}

impl MakiConfig {
    pub fn project_title(&self) -> Option<&str> {
        self.project_title.as_deref()
    }

    pub fn home_mode(&self) -> &HomeMode {
        &self.home_mode
    }

    pub fn publish_policy(&self) -> &PublishPolicy {
        &self.publish_policy
    }

    pub fn project_source_root(&self, project_root: &Path) -> PathBuf {
        project_root.join(&self.source_dir)
    }

    pub fn load_project(root: &Path) -> Result<Self, Error> {
        let project_file = root.join(PROJECT_FILE_NAME);

        if !project_file.exists() {
            return Ok(Self::default());
        }
        if !project_file.is_file() {
            return Err(Error::InvalidProjectFile(
                project_file,
                "expected a regular file".to_string(),
            ));
        }

        let raw = std::fs::read_to_string(&project_file)
            .map_err(|_source| Error::ReadProjectFileFailed(project_file.clone()))?;
        let project = ProjectToml::parse(&project_file, &raw)?;

        let mut config = Self {
            project_title: project.title,
            source_dir: project
                .source
                .map(|source| parse_project_source(&project_file, &source))
                .transpose()?
                .unwrap_or_else(|| PathBuf::from(".")),
            ..Default::default()
        };
        if let Some(home) = project.home {
            config.set_home_note_ref(home);
        }
        Ok(config)
    }

    pub fn set_home_redirect(&mut self, path: impl Into<String>) {
        self.home_mode = HomeMode::Redirect(path.into());
    }

    fn set_home_note_ref(&mut self, note_ref: impl AsRef<str>) {
        self.set_home_redirect(note_ref_to_redirect_path(note_ref.as_ref()));
    }
}

impl Default for MakiConfig {
    fn default() -> Self {
        Self {
            project_title: None,
            source_dir: PathBuf::from("."),
            home_mode: HomeMode::Redirect("/README".to_string()),
            publish_policy: PublishPolicy::PublishAll,
        }
    }
}

#[derive(Debug, PartialEq, Clone, Default)]
pub struct MakiConfigOverrides {
    home_redirect: Option<String>,
}

impl MakiConfigOverrides {
    pub fn from_home_redirect(home_redirect: Option<String>) -> Self {
        Self { home_redirect }
    }

    pub fn apply_to(&self, config: &mut MakiConfig) {
        if let Some(home_redirect) = &self.home_redirect {
            config.set_home_redirect(home_redirect.clone());
        }
    }
}

#[derive(Default)]
struct ProjectToml {
    title: Option<String>,
    source: Option<String>,
    home: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum TomlSection {
    Project,
    Other,
}

impl ProjectToml {
    fn parse(path: &Path, raw: &str) -> Result<Self, Error> {
        let mut project = Self::default();
        let mut section = TomlSection::Other;

        for (index, raw_line) in raw.lines().enumerate() {
            let line_number = index + 1;
            let line = trim_toml_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }

            if line.starts_with('[') {
                section = parse_toml_section(path, line_number, line)?;
                continue;
            }

            if section != TomlSection::Project {
                continue;
            }

            let Some((raw_key, raw_value)) = line.split_once('=') else {
                return Err(invalid_project_file(
                    path,
                    line_number,
                    "expected key = value",
                ));
            };
            match raw_key.trim() {
                "title" => {
                    project.title = Some(parse_toml_string_value(
                        path,
                        line_number,
                        raw_value.trim(),
                    )?)
                }
                "home" => {
                    project.home = Some(parse_toml_string_value(
                        path,
                        line_number,
                        raw_value.trim(),
                    )?)
                }
                "source" => {
                    project.source = Some(parse_toml_string_value(
                        path,
                        line_number,
                        raw_value.trim(),
                    )?)
                }
                _ => continue,
            }
        }

        Ok(project)
    }
}

fn parse_project_source(project_file: &Path, source: &str) -> Result<PathBuf, Error> {
    let path = Path::new(source);
    if source.is_empty() || path.components().any(is_outside_project_component) {
        return Err(Error::InvalidProjectFile(
            project_file.to_path_buf(),
            "project.source must be a relative path inside the project".to_string(),
        ));
    }

    Ok(path.to_path_buf())
}

fn is_outside_project_component(component: Component<'_>) -> bool {
    matches!(
        component,
        Component::ParentDir | Component::RootDir | Component::Prefix(_)
    )
}

fn note_ref_to_redirect_path(note_ref: &str) -> String {
    if note_ref.starts_with('/') {
        note_ref.to_string()
    } else {
        format!("/{note_ref}")
    }
}

fn parse_toml_section(path: &Path, line_number: usize, line: &str) -> Result<TomlSection, Error> {
    if !line.ends_with(']') {
        return Err(invalid_project_file(
            path,
            line_number,
            "unterminated table header",
        ));
    }

    let section = line
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or("")
        .trim();

    match section {
        "project" => Ok(TomlSection::Project),
        _ => Ok(TomlSection::Other),
    }
}

fn trim_toml_comment(line: &str) -> &str {
    let mut in_basic_string = false;
    let mut in_literal_string = false;
    let mut escaped = false;

    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if in_basic_string => escaped = true,
            '"' if !in_literal_string => in_basic_string = !in_basic_string,
            '\'' if !in_basic_string => in_literal_string = !in_literal_string,
            '#' if !in_basic_string && !in_literal_string => return &line[..index],
            _ => {}
        }
    }

    line
}

fn parse_toml_string_value(path: &Path, line_number: usize, raw: &str) -> Result<String, Error> {
    let Some(quote) = raw.chars().next() else {
        return Err(invalid_project_file(
            path,
            line_number,
            "expected string value",
        ));
    };

    match quote {
        '"' => parse_basic_toml_string(path, line_number, raw),
        '\'' => parse_literal_toml_string(path, line_number, raw),
        _ => Err(invalid_project_file(
            path,
            line_number,
            "expected string value",
        )),
    }
}

fn parse_literal_toml_string(path: &Path, line_number: usize, raw: &str) -> Result<String, Error> {
    let rest = &raw[1..];
    let Some(end) = rest.find('\'') else {
        return Err(invalid_project_file(
            path,
            line_number,
            "unterminated literal string",
        ));
    };
    let value = &rest[..end];
    reject_trailing_toml_string_content(path, line_number, &rest[end + 1..])?;
    Ok(value.to_string())
}

fn parse_basic_toml_string(path: &Path, line_number: usize, raw: &str) -> Result<String, Error> {
    let mut value = String::new();
    let mut escaped = false;

    for (index, ch) in raw[1..].char_indices() {
        if escaped {
            match ch {
                'n' => value.push('\n'),
                'r' => value.push('\r'),
                't' => value.push('\t'),
                '"' => value.push('"'),
                '\\' => value.push('\\'),
                _ => {
                    return Err(invalid_project_file(
                        path,
                        line_number,
                        "unsupported string escape",
                    ));
                }
            }
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '"' => {
                reject_trailing_toml_string_content(path, line_number, &raw[index + 2..])?;
                return Ok(value);
            }
            _ => value.push(ch),
        }
    }

    Err(invalid_project_file(
        path,
        line_number,
        "unterminated string",
    ))
}

fn reject_trailing_toml_string_content(
    path: &Path,
    line_number: usize,
    trailing: &str,
) -> Result<(), Error> {
    if trailing.trim().is_empty() {
        Ok(())
    } else {
        Err(invalid_project_file(
            path,
            line_number,
            "unexpected content after string value",
        ))
    }
}

fn invalid_project_file(path: &Path, line_number: usize, message: &str) -> Error {
    Error::InvalidProjectFile(path.to_path_buf(), format!("line {line_number}: {message}"))
}

#[derive(Debug, PartialEq, Clone)]
pub enum PublishPolicy {
    PublishAll,
    // TODO: TaggedOnly: publish 설정한 파일만 접근 가능하게 하기,
}

#[derive(Debug, PartialEq, Clone)]
pub enum HomeMode {
    Redirect(String),
}
