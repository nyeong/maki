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
