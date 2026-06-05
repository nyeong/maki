# TODO

## 1단계: 기본 웹 서빙

- [x] markdown target directory 받기
- [x] markdown 파일 인덱싱
- [x] 웹 서버 띄우기
- [x] `/n/{path}` 노트 라우팅
- [x] `.md` 요청은 원본 markdown 반환
- [x] 확장자 없는 요청은 HTML 응답으로 분기
- [x] HTTP 응답 구조화와 wire format 테스트
- [x] HTTP 타입, 파서, 응답 모듈 분리

## 2단계: 구조 분리

- [ ] web adapter 분리: `http::Request`를 Maki route로 해석하고 `http::Response`로 변환
- [ ] Maki 도메인 모듈 분리
- [ ] TCP server loop 모듈 분리
- [ ] `RunError::RequestParseError`를 `http::Error` 또는 도메인 에러로 정리

## 3단계: Markdown 렌더링

- [ ] `pulldown-cmark` 추가
- [ ] `render_markdown(source)` 구현
- [ ] `render_html`에서 raw markdown 대신 렌더링 결과 사용
- [ ] markdown 렌더링 fixture와 테스트 추가

## 4단계: Maki 확장과 색인

- [ ] `MakiDocument`와 `MakiAnnotation` 설계
- [ ] `SCHEDULED: <yyyy-mm-dd>` 파싱
- [ ] `/schedule/{date}` 라우트
- [ ] wikilink resolve
- [ ] deadlink 검사
- [ ] 백링크
- [ ] full text search
- [ ] git status 표시

## 5단계: 공유

- [ ] 공유용 publish 구분
