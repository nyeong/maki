use super::*;

#[test]
fn git_serve_polls_commits_without_live_reload() {
    if !git_is_available() {
        return;
    }

    let repo = temp_git_project("git-serve-repo");
    let state = temp_project("git-serve-state");
    let port = free_port();
    let _server = start_git_server(&repo.root, &state.root, port);

    let initial = http_get(port, "/home");
    initial.assert_status("HTTP/1.1 200 OK");
    initial.assert_body_contains("Git content marker: version one");
    initial.assert_body_excludes("new EventSource(\"/.maki/events\")");

    commit_git_project(&repo.root, "second", "Git content marker: version two");

    let updated = wait_until_body_contains(port, "/home", "Git content marker: version two");
    updated.assert_status("HTTP/1.1 200 OK");
    updated.assert_body_excludes("new EventSource(\"/.maki/events\")");
}

#[test]
fn git_serve_recents_use_commit_times() {
    if !git_is_available() {
        return;
    }

    let repo = temp_project("git-recents-repo");
    run_git(&repo.root, &["init", "--initial-branch", "main"]);
    run_git(&repo.root, &["config", "user.name", "Maki Test"]);
    run_git(
        &repo.root,
        &["config", "user.email", "maki-test@example.invalid"],
    );
    fs::create_dir_all(repo.root.join("docs")).unwrap();
    fs::write(
        repo.root.join("maki.toml"),
        "[project]\ntitle = \"Git Fixture\"\nsource = \"docs\"\nhome = \"old\"\n",
    )
    .unwrap();
    fs::write(
        repo.root.join("docs").join("old.maki"),
        "--^ title: Old Note\n\nOld body.\n",
    )
    .unwrap();
    commit_git_project_at(&repo.root, "old", "2001-01-01T00:00:00+0000");

    fs::write(
        repo.root.join("docs").join("new.maki"),
        "--^ title: New Note\n\nNew body.\n",
    )
    .unwrap();
    commit_git_project_at(&repo.root, "new", "2001-01-02T00:00:00+0000");

    let state = temp_project("git-recents-state");
    let port = free_port();
    let _server = start_git_server(&repo.root, &state.root, port);

    let recents = http_get(port, "/@/recents");
    recents.assert_status("HTTP/1.1 200 OK");
    recents.assert_body_contains("2001-01-02 09:00 KST");
    recents.assert_body_contains("2001-01-01 09:00 KST");
    assert_body_order(
        &recents.body,
        &[
            "2001-01-02 09:00 KST",
            "New Note",
            "2001-01-01 09:00 KST",
            "Old Note",
        ],
    );
}
