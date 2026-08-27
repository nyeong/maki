(() => {
  const SEARCH_INDEX_PATH = "/.maki/search-index.json";
  const SEARCH_PATH = "/.maki/search";
  const MAX_RESULTS = 8;
  const normalize = (value) => value.toLocaleLowerCase();
  const matchRank = (entry, query) => {
    const title = normalize(entry.title || "");
    if (title === query) return [0, 0, title.length];
    if (title.startsWith(query)) return [1, 0, title.length];
    const index = title.indexOf(query);
    return index === -1 ? null : [2, index, title.length];
  };
  const compareMatches = (left, right) => {
    for (let index = 0; index < left.rank.length; index += 1) {
      if (left.rank[index] !== right.rank[index]) {
        return left.rank[index] - right.rank[index];
      }
    }
    return (left.entry.title || "").localeCompare(right.entry.title || "");
  };
  const findMatches = (entries, rawQuery) => {
    const query = normalize(rawQuery.trim());
    if (!query) return [];
    const seenPaths = new Set();
    return entries
      .map((entry) => ({ entry, rank: matchRank(entry, query) }))
      .filter((match) => match.rank)
      .sort(compareMatches)
      .filter((match) => {
        if (seenPaths.has(match.entry.path)) return false;
        seenPaths.add(match.entry.path);
        return true;
      })
      .slice(0, MAX_RESULTS)
      .map((match) => match.entry);
  };
  const displayTitle = (entry) =>
    entry.kind === "heading" ? `#${entry.title}` : entry.title;

  if (typeof __makiSearchUnitTestExports === "function") {
    __makiSearchUnitTestExports({ displayTitle, findMatches });
  }

  const forms = document.querySelectorAll("[data-maki-search]");
  if (!forms.length) return;

  let entriesPromise;
  const loadEntries = () => {
    if (!entriesPromise) {
      entriesPromise = fetch(SEARCH_INDEX_PATH, {
        headers: { Accept: "application/json" },
      }).then((response) => (response.ok ? response.json() : []));
    }
    return entriesPromise;
  };

  forms.forEach((form) => {
    const input = form.querySelector("[data-maki-search-input]");
    const results = form.querySelector("[data-maki-search-results]");
    if (!input || !results) return;

    if (location.pathname === SEARCH_PATH && !input.value) {
      input.value = new URLSearchParams(location.search).get("q") || "";
    }

    let activeIndex = -1;
    let currentMatches = [];
    let requestId = 0;

    const close = () => {
      results.hidden = true;
      results.textContent = "";
      activeIndex = -1;
      currentMatches = [];
    };
    const setActive = (index) => {
      activeIndex = index;
      Array.from(results.children).forEach((child, childIndex) => {
        const selected = childIndex === activeIndex;
        child.classList.toggle("is-active", selected);
        child.setAttribute("aria-selected", selected ? "true" : "false");
      });
    };
    const createResultLink = (entry, index) => {
      const link = document.createElement("a");
      link.className = "maki-search-result";
      link.href = entry.path;
      link.setAttribute("role", "option");
      link.setAttribute("aria-selected", "false");
      link.addEventListener("mouseenter", () => setActive(index));

      const title = document.createElement("span");
      title.className = "maki-search-result-title";
      title.textContent = displayTitle(entry);
      const source = document.createElement("span");
      source.className = "maki-search-result-source";
      source.textContent = `${entry.kind || "note"}: ${entry.source_path}`;

      link.append(title, source);
      return link;
    };
    const render = (matches) => {
      results.textContent = "";
      currentMatches = matches;
      if (!matches.length) {
        close();
        return;
      }

      matches.forEach((entry, index) => {
        results.append(createResultLink(entry, index));
      });

      results.hidden = false;
      setActive(0);
    };
    const update = async () => {
      const id = ++requestId;
      const query = input.value.trim();
      if (!query) {
        close();
        return;
      }
      const entries = await loadEntries();
      if (id !== requestId) return;
      render(findMatches(entries, query));
    };

    input.addEventListener("input", update);
    input.addEventListener("focus", () => {
      if (input.value.trim()) update();
    });
    input.addEventListener("keydown", (event) => {
      if (!currentMatches.length) return;
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setActive((activeIndex + 1) % currentMatches.length);
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        setActive(
          (activeIndex - 1 + currentMatches.length) % currentMatches.length,
        );
      } else if (event.key === "Enter") {
        event.preventDefault();
        location.href = currentMatches[Math.max(activeIndex, 0)].path;
      } else if (event.key === "Escape") {
        close();
      }
    });
    form.addEventListener("submit", async (event) => {
      const query = input.value.trim();
      if (!query) return;
      event.preventDefault();
      const matches = findMatches(await loadEntries(), query);
      location.href = matches.length
        ? matches[0].path
        : `${SEARCH_PATH}?q=${encodeURIComponent(query)}`;
    });
    document.addEventListener("click", (event) => {
      if (!form.contains(event.target)) close();
    });
  });
})();
