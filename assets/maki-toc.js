(() => {
  const HEADING_SELECTOR =
    "h2, h3, h4, h5, h6, [role='heading'][aria-level]";
  const DEFAULT_CONTENT_HEADING_LEVEL = 3;
  const MIN_CONTENT_HEADING_LEVEL = 1;
  const ACTIVE_ANCHOR_RATIO = 0.32;
  const ACTIVE_ANCHOR_MAX = 260;
  const MARKER_PRECISION = 3;
  const MARKER_OVERLAP_GAP = 2;

  const getHeadingLevel = (heading) => {
    const ariaLevel = Number(heading.getAttribute("aria-level"));
    if (Number.isInteger(ariaLevel) && ariaLevel > 1) return ariaLevel;

    const match = heading.tagName.match(/^H([2-6])$/);
    return match ? Number(match[1]) : 6;
  };

  const getContentHeadingLevel = (heading) =>
    Math.max(getHeadingLevel(heading) - 1, MIN_CONTENT_HEADING_LEVEL);

  const collectHeadings = () =>
    Array.from(document.querySelectorAll(HEADING_SELECTOR)).filter(
      (heading) =>
        !heading.closest(".maki-toc") && heading.textContent.trim(),
    );

  const numberedLabels = (headings) => {
    const minLevel = Math.min(...headings.map(getHeadingLevel));
    const counters = [];

    return headings.map((heading) => {
      const depth = Math.max(getHeadingLevel(heading) - minLevel, 0);
      counters.length = Math.min(counters.length, depth + 1);

      while (counters.length < depth) {
        counters.push(1);
      }
      while (counters.length <= depth) {
        counters.push(0);
      }

      counters[depth] += 1;
      return `${counters.join(".")}. ${heading.textContent.trim()}`;
    });
  };

  const headingTop = (heading) =>
    heading.getBoundingClientRect().top + globalThis.scrollY;

  const scrollRange = () =>
    Math.max(
      document.documentElement.scrollHeight,
      document.body?.scrollHeight ?? 0,
    ) - globalThis.innerHeight;

  const markerPosition = (heading, range) => {
    if (range <= 0) return 0;

    return Math.min(Math.max(headingTop(heading) / range, 0), 1);
  };

  const markerBoxOverlaps = (box, otherBox) =>
    box.top < otherBox.bottom && box.bottom > otherBox.top;

  const markerBoxPriority = (left, right) =>
    left.level - right.level || left.index - right.index;

  const overlappedMarkerIndexes = (markerBoxes) => {
    const visibleBoxes = [];
    const overlappedIndexes = new Set();

    [...markerBoxes].sort(markerBoxPriority).forEach((box) => {
      const overlapped = visibleBoxes.some((visibleBox) =>
        markerBoxOverlaps(box, visibleBox),
      );

      if (overlapped) {
        overlappedIndexes.add(box.index);
      } else {
        visibleBoxes.push(box);
      }
    });

    return overlappedIndexes;
  };

  const insertToc = (toc) => {
    const title = document.querySelector("body > h1");
    if (title) {
      title.insertAdjacentElement("afterend", toc);
      return;
    }

    const nav = document.querySelector("body > .maki-nav");
    if (nav) {
      nav.insertAdjacentElement("afterend", toc);
      return;
    }

    document.body.prepend(toc);
  };

  const chooseHeadings = (headings) => {
    const visibleDepth = headings.filter(
      (heading) =>
        getContentHeadingLevel(heading) <= DEFAULT_CONTENT_HEADING_LEVEL,
    );

    return visibleDepth.length ? visibleDepth : headings;
  };

  const labelParts = (labelText) => {
    const match = labelText.match(/^(\d+(?:\.\d+)*\.)(?:\s+)?(.*)$/);
    return match
      ? { number: match[1], title: match[2] }
      : { number: "", title: labelText };
  };

  const activeAnchorY = () =>
    globalThis.scrollY +
    Math.min(globalThis.innerHeight * ACTIVE_ANCHOR_RATIO, ACTIVE_ANCHOR_MAX);

  const start = () => {
    if (document.querySelector(".maki-toc")) return;

    const headings = chooseHeadings(collectHeadings());
    if (!headings.length) return;

    const toc = document.createElement("nav");
    toc.className = "maki-toc";
    toc.setAttribute("aria-label", "Section map");

    const panel = document.createElement("details");
    panel.className = "maki-toc-panel";
    panel.open = true;
    const summary = document.createElement("summary");
    summary.className = "maki-toc-summary";
    summary.textContent = "목차";
    const list = document.createElement("ol");
    list.className = "maki-toc-list";
    panel.append(summary, list);
    toc.append(panel);
    insertToc(toc);

    const markerMap = document.createElement("nav");
    markerMap.className = "maki-toc-map";
    markerMap.setAttribute("aria-label", "Section scroll map");
    const markerList = document.createElement("ol");
    markerList.className = "maki-toc-map-list";
    markerMap.append(markerList);
    document.body.append(markerMap);
    markerMap.addEventListener("mouseleave", () => {
      markerMap.classList.remove("is-expanded");
    });

    const setLinkTarget = (link, heading, labelText) => {
      link.href = heading.id ? `#${encodeURIComponent(heading.id)}` : "#";
      link.setAttribute("aria-label", labelText);
      link.addEventListener("click", (event) => {
        event.preventDefault();
        heading.scrollIntoView({ block: "start" });
        if (heading.id) {
          globalThis.history.replaceState(
            null,
            "",
            `${globalThis.location.pathname}${globalThis.location.search}#${encodeURIComponent(heading.id)}`,
          );
        }
        scheduleActiveUpdate();
      });
    };

    const appendLabelContent = (target, parts, numberClass) => {
      if (parts.number) {
        const number = document.createElement("span");
        number.className = numberClass;
        const nestedNumber = parts.number.replace(/\.$/, "");
        number.textContent = `${nestedNumber.includes(".") ? nestedNumber : parts.number} `;
        target.append(number);
      }
      target.append(document.createTextNode(parts.title));
    };

    const labelTexts = numberedLabels(headings);
    const items = headings.map((heading, index) => {
      const labelText = labelTexts[index];
      const parts = labelParts(labelText);
      const item = document.createElement("li");
      item.className = "maki-toc-item";
      item.style.setProperty(
        "--maki-toc-depth",
        Math.max(getContentHeadingLevel(heading) - 1, 0).toString(),
      );

      const link = document.createElement("a");
      setLinkTarget(link, heading, labelText);
      appendLabelContent(link, parts, "maki-toc-list-number");
      item.append(link);
      list.append(item);

      const markerItem = document.createElement("li");
      markerItem.className = "maki-toc-map-item";
      const markerLink = document.createElement("a");
      markerLink.className = "maki-toc-map-link";
      setLinkTarget(markerLink, heading, labelText);

      const markerLabel = document.createElement("span");
      markerLabel.className = "maki-toc-map-label";
      markerLabel.addEventListener("mouseenter", () => {
        markerMap.classList.add("is-expanded");
      });
      appendLabelContent(markerLabel, parts, "maki-toc-map-number");
      markerLink.append(markerLabel);
      markerItem.append(markerLink);
      markerList.append(markerItem);

      return { heading, item, link, markerItem, markerLink };
    });

    let activeFrame = 0;
    let markerFrame = 0;
    let activeIndex = -1;

    const visibleHeadingIndex = () =>
      items.findIndex(({ heading }) => {
        const rect = heading.getBoundingClientRect();
        return rect.top < globalThis.innerHeight && rect.bottom > 0;
      });

    const previousHeadingIndex = () => {
      const anchorY = activeAnchorY();

      for (let index = items.length - 1; index >= 0; index -= 1) {
        if (headingTop(items[index].heading) <= anchorY) return index;
      }

      return -1;
    };

    const findActiveIndex = () => {
      const visibleIndex = visibleHeadingIndex();
      return visibleIndex >= 0 ? visibleIndex : previousHeadingIndex();
    };

    const setActiveIndex = (nextActiveIndex) => {
      if (activeIndex === nextActiveIndex) return;

      activeIndex = nextActiveIndex;
      items.forEach(({ item, link, markerItem, markerLink }, index) => {
        const active = index === activeIndex;
        item.classList.toggle("is-active", active);
        markerItem.classList.toggle("is-active", active);
        if (active) {
          link.setAttribute("aria-current", "location");
          markerLink.setAttribute("aria-current", "location");
        } else {
          link.removeAttribute("aria-current");
          markerLink.removeAttribute("aria-current");
        }
      });
    };

    function scheduleActiveUpdate() {
      if (activeFrame) return;

      activeFrame = globalThis.requestAnimationFrame(() => {
        activeFrame = 0;
        setActiveIndex(findActiveIndex());
      });
    }

    const updateMarkerPositions = () => {
      const range = scrollRange();
      const listRect = markerList.getBoundingClientRect();

      const markerBoxes = items.map(({ heading, markerItem, markerLink }, index) => {
        const position = markerPosition(heading, range);
        markerItem.style.setProperty(
          "--maki-toc-marker-y",
          `${(position * 100).toFixed(MARKER_PRECISION)}%`,
        );
        markerItem.classList.remove("is-overlapped");

        const height = Math.max(markerLink.getBoundingClientRect().height, 1);
        const center = listRect.top + listRect.height * position;
        return {
          bottom: center + height / 2 + MARKER_OVERLAP_GAP,
          index,
          level: getContentHeadingLevel(heading),
          top: center - height / 2 - MARKER_OVERLAP_GAP,
        };
      });

      if (listRect.height <= 0) return;

      const overlappedIndexes = overlappedMarkerIndexes(markerBoxes);
      items.forEach(({ markerItem }, index) => {
        markerItem.classList.toggle("is-overlapped", overlappedIndexes.has(index));
      });
    };

    function scheduleMarkerUpdate() {
      if (markerFrame) return;

      markerFrame = globalThis.requestAnimationFrame(() => {
        markerFrame = 0;
        updateMarkerPositions();
      });
    }

    const scheduleLayoutUpdate = () => {
      scheduleMarkerUpdate();
      scheduleActiveUpdate();
    };

    scheduleLayoutUpdate();
    globalThis.requestAnimationFrame(() => toc.classList.add("is-ready"));
    globalThis.addEventListener("load", scheduleLayoutUpdate, {
      passive: true,
    });
    globalThis.addEventListener("resize", scheduleLayoutUpdate, {
      passive: true,
    });
    globalThis.addEventListener("scroll", scheduleActiveUpdate, {
      passive: true,
    });
  };

  const startWhenReady = () => {
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", start, { once: true });
    } else {
      start();
    }

    globalThis.addEventListener("load", start, { once: true });
    globalThis.addEventListener("resize", start, { passive: true });
  };

  if (globalThis.__makiTocUnitTestExports) {
    globalThis.__makiTocUnitTestExports({
      markerBoxOverlaps,
      overlappedMarkerIndexes,
    });
    return;
  }

  startWhenReady();
})();
