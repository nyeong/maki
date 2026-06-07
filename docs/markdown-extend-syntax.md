---
title: Markdown Extend syntax
---

# note identity

1. root-relative file path를 note identity로 삼음
   - 예시) 파일 경로가 `notes/esperanto.md`면
     - 웹 path는 `/notes/esperanto`
     - [[#wikilink]]는 `[[esperanto]]` 또는 `[[notes/esperanto]]`
     - display name은 `esperanto`
       - 단 `frontmatter.title`로 override 할 수 있음
1. case-insensitive하게 unique해야함
   - 예시
     - `Esperanto.md`와 `esperanto.md`는 동시에 존재할 수 없음.

# heading

- 내부에서 헤딩을 어떻게 쓸 지는 자유이나, lv1(`#`)부터 쓰는 것을 권장함

# wikilink

## Resolve Rule

1. exact path

   ```
   [[notes/esperanto]] -> notes/esperanto.md
   ```

2. same-directory filename stem

   ```
   [[esperanto]] -> notes/esperanto.md
   ```

   `notes/esperanto.md`, `languages/esperanto.md`처럼 같은 파일 이름인 두 경로가 있어서 모호한 경우, 현재 파일 기준 sibling을 우선함

   예

   ```text
   notes/a.md
   notes/esperanto.md
   languages/esperanto.md
   ```

   위의 상황에서, `notes/a.md`의 `[[esperanto]]`는 `notes/esperanto.md`를 의미한다.

3. project-wide filename stem

   (2)에서 후보가 없으면 프로젝트 전체에서 같은 stem을 찾는다.
   - 후보가 하나면 resolve
   - 후보가 여러 개면 ambiguous link
   - 후보가 없으면 broken link

4. otherwise -> broken link
