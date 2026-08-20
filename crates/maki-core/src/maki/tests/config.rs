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
