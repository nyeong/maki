use super::*;

#[test]
fn project_config_can_set_source_directory() {
    let project = temp_project("project-source");
    fs::write(
        project.root.join(PROJECT_FILE_NAME),
        "[project]\ntitle = \"Source Fixture\"\nsource = \"docs\"\nhome = \"index\"\n",
    )
    .unwrap();

    let config = MakiConfig::load_project(&project.root).unwrap();

    assert_eq!(
        config.project_source_root(&project.root),
        project.root.join("docs")
    );
    assert_eq!(
        config.home_mode(),
        &HomeMode::Redirect("/index".to_string())
    );
}

#[test]
fn project_config_can_set_serve_favicon() {
    let project = temp_project("project-favicon");
    fs::write(
        project.root.join(PROJECT_FILE_NAME),
        "[project]\ntitle = \"Favicon Fixture\"\n\n[serve]\nfavicon = \"assets/favicon.png\"\n",
    )
    .unwrap();

    let config = MakiConfig::load_project(&project.root).unwrap();

    assert_eq!(config.favicon(), Some(Path::new("assets/favicon.png")));
    assert_eq!(config.favicon_content_type(), Some("image/png"));
}

#[test]
fn project_config_rejects_source_outside_project() {
    let project = temp_project("project-source-invalid");
    fs::write(
        project.root.join(PROJECT_FILE_NAME),
        "[project]\nsource = \"../docs\"\n",
    )
    .unwrap();

    assert!(matches!(
        MakiConfig::load_project(&project.root),
        Err(Error::InvalidProjectFile(_, message))
            if message == "project.source must be a relative path inside the project"
    ));
}

#[test]
fn project_config_rejects_favicon_outside_project() {
    let project = temp_project("project-favicon-invalid");
    fs::write(
        project.root.join(PROJECT_FILE_NAME),
        "[serve]\nfavicon = \"../favicon.png\"\n",
    )
    .unwrap();

    assert!(matches!(
        MakiConfig::load_project(&project.root),
        Err(Error::InvalidProjectFile(_, message))
            if message == "serve.favicon must be a relative path inside the project"
    ));
}

#[test]
fn project_config_rejects_unsupported_favicon_type() {
    let project = temp_project("project-favicon-invalid-type");
    fs::write(
        project.root.join(PROJECT_FILE_NAME),
        "[serve]\nfavicon = \"assets/favicon.txt\"\n",
    )
    .unwrap();

    assert!(matches!(
        MakiConfig::load_project(&project.root),
        Err(Error::InvalidProjectFile(_, message))
            if message == "serve.favicon must be a PNG, SVG, ICO, WebP, or JPEG file"
    ));
}
