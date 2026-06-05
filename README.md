# Maki

File based Markdown wiki runtime.

## 개요

Emacs + org를 대체하기 위해 에디터 + 마크다운 LSP를 이용하는 경우, LSP만으로는 부족한 영역을 메우기 위한 프로젝트

## 목표

- 우이동 장인의 한땀한땀 정성어린 수제 코딩
- 웹 서빙
  - 공유용 임시 링크 생성 (비밀번호형, 횟수제한형, 시간제한형, 이메일 OTP형, ...)
- 캘린더 서빙 : 스케줄, 데드라인 등 캘린더 관련
- 린팅
  - 죽은 링크 검사
  - 위키링크 검사
- 컴파일 : 입력받은 마크다운을 컴파일하여 가상 버퍼를 만듭니다
  - 예) 태그 리스팅 등
- 위 기능들을 위한 확장 문법

## 목표가 아닌 것

## 구현 현황

[docs/todo.md](./docs/todo.md) 참고

## Development

```
cargo llvm-cov --text --show-missing-lines
cargo llvm-cov --html
```
