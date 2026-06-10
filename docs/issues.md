---
title: Issues
---

# wikilink가 제대로 resolve 되지 않음

@status=TODO, @type=bug

예를 들어 아래처럼 파일이 있을 때:

    README.md
    notes/홈랩.md

`/README`로 접속하면 아래처럼 렌더링됨

```html
<ul>
  <li><a href="%ED%99%88%EB%9E%A9">홈랩</a></li>
</ul>
```

- 근데 실제로는 `/notes/홈랩.md`로 링크가 걸려야함. `/notes` 하위 경로가 무시됨.
- 단순 변환이 아니라 대상의 canonical route로 바꾸도록

# TODO 문법 정하기

@status=TODO, @type=syntax

- 무엇에
  - 헤딩에 (org처럼)
  - 인라인에
- 어떻게
  - 헤딩 앞에 `TODO` (org처럼)
    - `[[]]` 위키 링크 문법으로 링크 걸 때, TODO가 안에 포함됨
  - 속성으로
    - 속성 문법을 정의해야
- 대체는
  - 그냥 태그로 대체하면?

# Date/Schedule 문법 정하기

@status=TODO, @type=syntax

- date range
- deadline
- repeat
- ...

# 숨김 파일을 traversal 하지 않도록

@status=TODO, @type=bug

현재 `serve`하면 `.direnv`와 같은 숨김 폴더에서도 `.md`를 찾음.

- 숨김 폴더의 기준은 뭐로?
