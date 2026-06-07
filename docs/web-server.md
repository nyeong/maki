# Web Server

- `/` : 루트 (지금은 redirect만 지원)
- `/{note-path}` : 렌더링 된 마크다운 파일
- `/{note-path}.md` : 원본 마크다운 파일
- `/{directory-path}/` : 404
- `/@/{path}` : 동적 페이지들
  - 예: 태그, 스케줄 등 동적으로 모은 데이터들을 보여줄 곳
    - `/@/tags`, `/@/tags/{tag}`, ...
