use super::*;

#[test]
fn project_config_can_set_source_directory() {
    let config = MakiConfig::parse(
        Path::new(PROJECT_FILE_NAME),
        "[project]\ntitle = \"Source Fixture\"\nsource = \"docs\"\nhome = \"index\"\n",
    )
    .unwrap();

    assert_eq!(
        config.project_source_root(Path::new("project")),
        PathBuf::from("project/docs")
    );
    assert_eq!(config.project_title(), Some("Source Fixture"));
    assert_eq!(
        config.home_mode(),
        &HomeMode::Redirect("/index".to_string())
    );
}

#[test]
fn project_config_can_set_serve_favicon() {
    let config = MakiConfig::parse(
        Path::new(PROJECT_FILE_NAME),
        "[project]\ntitle = \"Favicon Fixture\"\n\n[serve]\nfavicon = \"assets/favicon.png\"\n",
    )
    .unwrap();

    assert_eq!(config.favicon(), Some(Path::new("assets/favicon.png")));
    assert_eq!(config.favicon_content_type(), Some("image/png"));
}

#[test]
fn project_config_rejects_source_outside_project() {
    assert!(matches!(
        MakiConfig::parse(
            Path::new(PROJECT_FILE_NAME),
            "[project]\nsource = \"../docs\"\n"
        ),
        Err(Error::InvalidProjectFile(_, message))
            if message == "project.source must be a relative path inside the project"
    ));
}

#[test]
fn project_config_rejects_favicon_outside_project() {
    assert!(matches!(
        MakiConfig::parse(
            Path::new(PROJECT_FILE_NAME),
            "[serve]\nfavicon = \"../favicon.png\"\n"
        ),
        Err(Error::InvalidProjectFile(_, message))
            if message == "serve.favicon must be a relative path inside the project"
    ));
}

#[test]
fn project_config_rejects_unsupported_favicon_type() {
    assert!(matches!(
        MakiConfig::parse(
            Path::new(PROJECT_FILE_NAME),
            "[serve]\nfavicon = \"assets/favicon.txt\"\n"
        ),
        Err(Error::InvalidProjectFile(_, message))
            if message == "serve.favicon must be a PNG, SVG, ICO, WebP, or JPEG file"
    ));
}
