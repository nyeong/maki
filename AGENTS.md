# 코드 작성 시

- premature abstraction 넣지말 것
- IO랑 sans IO 구분할 것
- sans IO에 유닛테스트 반드시 짤 것

# 작업 후

1. [subagent] Code Simplifier
2. [subagent] docs 정합성 확인
3. 의미 단위로 커밋
4. 사용자 승인 받고 push & deploy

{[1] [2]} -> 3 -> 4로 진행할 것.
subagent 썼으면 닫을것.

# docs 정합성 확인

[docs/]의 `*.maki` 문서들과 수정한 내용 간의 차이가 있는지 확인하고, 필요시 수정하여 이번 커밋에 포함

# push

```bash
nix develop -c git push nixbox main
```

# deploy

```bash
nix flake update maki --flake ~/.dotfiles
nix run nixpkgs#deploy-rs -- /Users/nyeong/.dotfiles/#nixbox -s --remote-build
```
