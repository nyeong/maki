use super::*;

#[test]
fn maki_serve_discovers_project_root_from_maki_toml() {
    let (_project, notes) = temp_project_with_maki_toml("serve-project-root");

    let port = free_port();
    let _server = start_server_with_project_config(&notes, port);

    let home = http_get(port, "/");
    home.assert_status("HTTP/1.1 302 Found");
    home.assert_header_contains("location: /start");

    let root_page = http_get(port, "/start");
    root_page.assert_status("HTTP/1.1 200 OK");

    let nested_page = http_get(port, "/notes/page");
    nested_page.assert_status("HTTP/1.1 200 OK");
    nested_page.assert_body_contains("<a href=\"/start\">Start</a>");
}

#[test]
fn maki_serve_uses_project_source_from_maki_toml() {
    let project = temp_project("serve-project-source");
    let docs = project.root.join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(
        project.root.join("maki.toml"),
        "[project]\ntitle = \"Source Fixture\"\nsource = \"docs\"\nhome = \"index\"\n",
    )
    .unwrap();
    fs::write(
        docs.join("index.maki"),
        "--^ title: Source Index\n\nProject source root.\n",
    )
    .unwrap();

    let port = free_port();
    let _server = start_server_with_project_config(&project.root, port);

    let home = http_get(port, "/");
    home.assert_status("HTTP/1.1 302 Found");
    home.assert_header_contains("location: /index");

    let index = http_get(port, "/index");
    index.assert_status("HTTP/1.1 200 OK");
    index.assert_body_contains("<title>Source Index</title>");

    let nested_path = http_get(port, "/docs/index");
    nested_path.assert_status("HTTP/1.1 404 Not Found");
}

#[test]
fn v0_fixture_serves_core_poc_behavior() {
    let port = free_port();
    let root = fixture_path("tests/fixtures/v0");
    let _server = start_server(&root, port, "/README");

    let home = http_get(port, "/");
    home.assert_status("HTTP/1.1 302 Found");
    home.assert_header_contains("location: /README");

    let page = http_get(port, "/README");
    page.assert_status("HTTP/1.1 200 OK");
    page.assert_body_contains("<title>v0 Quality Fixture</title>");
    page.assert_body_contains("<a href=\"/daily\">Daily</a>");
    page.assert_body_contains("<a href=\"/nested/홈랩\">홈랩</a>");
    page.assert_body_contains("<span class=\"broken-link\">ghost</span>");
    page.assert_body_contains("<span class=\"ambiguous-link\">ambiguous</span>");
    page.assert_body_contains(
        "<ul><li>top<ul><li>child<ul><li>grandchild</li></ul></li><li>sibling</li></ul></li><li>second top</li></ul>"
    );
    page.assert_body_contains(
        "<ol><li>supported ordered list</li><li>preserve this ordered shape</li></ol>",
    );
    page.assert_body_contains(
        "<pre><code class=\"language-html\">&lt;main&gt;\n  fixture code\n&lt;/main&gt;</code></pre>"
    );
    page.assert_body_excludes("should-not-render-property");

    let source = http_get(port, "/README.maki");
    source.assert_status("HTTP/1.1 200 OK");
    source.assert_body_contains("--^ hidden: should-not-render-property");
    source.assert_body_excludes("new EventSource");

    let unicode_page = http_get(port, "/nested/%ED%99%88%EB%9E%A9");
    unicode_page.assert_status("HTTP/1.1 200 OK");
    unicode_page.assert_body_contains("<title>홈랩</title>");

    let ignored_file = http_get(port, "/ignore");
    ignored_file.assert_status("HTTP/1.1 404 Not Found");
    ignored_file.assert_body_contains("<title>Not Found</title>");
    ignored_file.assert_body_contains("<header class=\"maki-nav\">");
    ignored_file.assert_body_contains("<a class=\"maki-home-link\" href=\"/\">/</a>");
}

#[test]
fn v1_fixture_supports_serve_options_and_live_reload() {
    let project = temp_project_from_fixture("v1-live-reload", "tests/fixtures/v1");
    let port = free_port();
    let _server = start_server(&project.root, port, "/home");

    let home = http_get(port, "/");
    home.assert_status("HTTP/1.1 302 Found");
    home.assert_header_contains("location: /home");

    let initial = http_get(port, "/home");
    initial.assert_status("HTTP/1.1 200 OK");
    initial.assert_body_contains("Initial content marker: v1-initial");
    initial.assert_body_contains("new EventSource(\"/.maki/events\")");

    let mut events = open_sse(port);
    read_until_contains(&mut events, "event: hello\ndata: ", Duration::from_secs(2));

    fs::write(
        project.root.join("home.maki"),
        "--^ title: v1 Live Reload Fixture\n\n= Status\n\nEdited content marker: v1-edited\n",
    )
    .unwrap();

    read_until_contains(&mut events, "event: reload\ndata: ", Duration::from_secs(5));

    let edited = http_get(port, "/home");
    edited.assert_body_contains("Edited content marker: v1-edited");
}
