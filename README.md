# Maki

Line-based lightweight mark-up language and file based personal wiki runtime.

## Goals

- 텍스트 기반 불렛 저널
- Emacs 밖에서 org스러운 사용성
- 개인 위키
- ICS, Reminder export
- 이력서 작성에 활용하기
- 내 개인적인 목적을 대상으로 Notion, org를 대체하는 것
- 등등

## 비목표

- Notion, org를 완전히 대체하는 것
- 협업
- 인터렉티브 UI
- 특정 문법과 완전 호환
- ...

## 참고

- 문법: [maki-syntax](docs/maki-syntax.maki)

## Development

```
cargo llvm-cov --text --show-missing-lines
cargo llvm-cov --html
```
