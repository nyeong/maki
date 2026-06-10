---
title: TODO
---

# 1단계: 기본 웹 서빙

- [x] markdown target directory 받기
- [x] markdown 파일 인덱싱
- [x] 웹 서버 띄우기
- [x] `/{path}` canonical note path 라우팅
- [x] `.md` 요청은 원본 markdown 반환
- [x] 확장자 없는 요청은 HTML 응답으로 분기
- [x] HTTP 응답 구조화와 wire format 테스트
- [x] HTTP 타입, 파서, 응답 모듈 분리

# 2단계: 구조 분리

- [x] web adapter 분리: `http::Request`를 Maki route로 해석하고 `http::Response`로 변환
- [ ] Maki 도메인 모듈 분리
- [ ] TCP server loop 모듈 분리
- [ ] `RunError::RequestParseError`를 `http::Error` 또는 도메인 에러로 정리

# 3단계: Markdown 렌더링

- [x] `pulldown-cmark` 추가
- [x] 기본 렌더링 함수 구현
- [x] `render_html`에서 raw markdown 대신 렌더링 결과 사용
- [ ] markdown 렌더링 fixture와 테스트 추가

# 4단계: Maki 확장, 인덱싱

- [x] note identity, wikilink resolve 규칙 정하기
  - [ ] wikilink가 resolve되는 게 아니라 일괄변환 되는 문제 수정하기
    - 예)
- [ ] 확장 문법 정하기
- [ ] 동적 페이지 정하기 (`/@/schedule/{date}`, TODO 등)
- [ ] case-insensitive unique note path
- [ ] wikilink resolve
- [ ] ambiguous wikilink 검사
- [ ] deadlink 검사
- [ ] 백링크
- [ ] full text search
- [ ] git status 표시

# 5단계: 공유

- [ ] 공유용 publish 구분
