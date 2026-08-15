# Maki

Line-based lightweight mark-up language and file based personal wiki runtime.

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

## NixOS

Maki can run multiple named targets from the NixOS module. Each target becomes
its own `maki-<name>.service`.

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
subdirectory.

## Development

```
cargo llvm-cov --text --show-missing-lines
cargo llvm-cov --html
```
