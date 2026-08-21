# Maki

Line-based lightweight mark-up language and file based personal wiki runtime.

## 현재 동작

- `.maki` 파일을 프로젝트 단위로 읽고 HTML로 렌더링한다.
- `maki.toml`의 `[project] title`, `source`, `home`을 읽고, `title`이 있으면 serve HTML title을 `page title | project title`로 렌더링한다.
- `[[note]]`, `[title](note)`, `[title](https://...)`, plain HTTP/HTTPS URL을 링크로 렌더링한다.
- wikilink는 exact path를 우선하고, 그 다음 case-insensitive path/stem lookup을 사용한다.
- paragraph, heading, property, quote, code, unordered/ordered list, hyphen-fenced container를 파싱한다.
- project page에는 home, meta, title search와 섹션 ToC가 있는 navigation shell이 붙는다.
- 충분히 넓은 화면에서는 ToC가 왼쪽 overlay로 고정되고, 현재 화면의 섹션만 강조된다.
- 데스크탑 1-column 화면에서는 ToC가 오른쪽 스크롤 위치 지도처럼 표시되고, 점 hover/focus 시 섹션 이름을 보여주며 클릭하면 해당 heading으로 이동한다.
- `/@/diagnostics`에서 diagnostics를 보고, `/@/dates`에서 date index를 보며, `/.maki/search`와 `/.maki/search-index.json`에서 title search를 쓸 수 있다.
- local serve는 live reload를 지원한다.
- `serve --git`는 git repository를 mirror/checkout하고 주기적으로 branch를 poll한다.
- `serve --metrics HOST:PORT`는 별도 listener에서 Prometheus `/metrics` endpoint를 제공한다.

## Goals

- 텍스트 기반 불렛 저널
- Emacs 밖에서 org스러운 사용성
- 개인 위키
- ICS, Reminder export
- 이력서 작성에 활용하기
- 등등

## 비목표

- Notion, org를 완전히 대체하는 것
- 협업
- 인터렉티브 UI
- 특정 문법과 완전 호환
- ...

## 참고

- 문법: [maki-syntax](docs/maki-syntax.maki)
- 현재 구현 분석: [implementation-analysis](docs/implementation-analysis.maki)

## Usage

```bash
maki serve docs
maki build docs/index.maki
maki serve --git https://example.invalid/maki.git --branch main --state-dir /var/lib/maki/docs --fetch-interval 60s
maki serve docs --metrics 127.0.0.1:4041
```

`maki serve <path>`는 `<path>` 또는 상위 디렉터리에서 `maki.toml`을 찾는다.
찾으면 project root와 configured `source`를 기준으로 serve하고, 찾지 못하면 기존처럼 입력 directory를 root로 본다.

`maki build <file>`도 같은 방식으로 project root를 찾는다.
파일이 configured source root 안에 있으면 project-aware link resolution과 diagnostics를 사용하고, 아니면 standalone HTML render로 떨어진다.

## maki.toml

```toml
[project]
title = "My Maki Notes"
source = "docs"
home = "index"
```

`source`는 project 내부 relative path여야 한다.
`home`은 note ref 기준이며, leading slash가 없으면 `/`가 붙은 redirect target으로 사용된다.
`title`은 serve HTML의 browser title suffix로 사용된다. 예를 들어 page title이 `Home`이면 `<title>Home | My Maki Notes</title>`가 된다. `title`이 없으면 기존처럼 page title만 사용한다.

## Web routes

- `/`: configured home으로 redirect
- `/<note>`: rendered note page
- `/<note>.maki`: source text
- `/@/`: meta index
- `/@/diagnostics`: diagnostics page
- `/@/dates`: date index
- `/@/dates/<date>`: date backlinks
- `/.maki/search`: title search page
- `/.maki/search-index.json`: title search index
- `/.maki/assets/...`: runtime CSS/JS assets

## Metrics

`maki serve --metrics 127.0.0.1:4041` enables a separate Prometheus text
exposition listener. Only `GET /metrics` is served on that listener. Metrics
use low-cardinality labels such as `route`, `kind`, `status`, `cache`,
`source`, and `result`; note paths, query strings, git commit hashes, raw
errors, and document content are not exported as labels.

## NixOS

Maki can run multiple named targets from the NixOS module. Each target becomes
its own `maki-<name>.service`. A target can serve either a local `source` path
or a git repository.

```nix
{
  inputs.maki.url = "git+https://example.invalid/maki.git";

  outputs =
    { maki, nixpkgs, ... }:
    {
      nixosConfigurations.nixbox = nixpkgs.lib.nixosSystem {
        modules = [
          maki.nixosModules.default
          {
            services.maki = {
              enable = true;

              targets.docs = {
                git.url = "https://example.invalid/maki.git";
                port = 4000;
                metrics.port = 4041;
                openFirewall = true;
              };
            };
          }
        ];
      };
    };
}
```

Git targets default to `branch = "main"`, `fetchInterval = "60s"`, and
`stateDir = "/var/lib/maki/<target-name>"`. Put `maki.toml` at the repository
root and use `source = "docs"` when the served Maki project lives in a
subdirectory. Metrics listeners default to `host = "127.0.0.1"` when
`metrics.port` is set.

For a local source target, set `source` instead of `git`:

```nix
{
  services.maki.targets.local = {
    source = "/srv/maki";
    port = 4000;
  };
}
```

## Development

```
cargo llvm-cov --text --show-missing-lines
cargo llvm-cov --html
```
