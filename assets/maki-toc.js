(() => {
  const HEADING_SELECTOR =
    "h2, h3, h4, h5, h6, [role='heading'][aria-level]";
  const DEFAULT_CONTENT_HEADING_LEVEL = 3;
  const MIN_CONTENT_HEADING_LEVEL = 1;
  const ACTIVE_ANCHOR_RATIO = 0.32;
  const ACTIVE_ANCHOR_MAX = 260;

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
      link.href = heading.id ? `#${encodeURIComponent(heading.id)}` : "#";
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

      if (parts.number) {
        const number = document.createElement("span");
        number.className = "maki-toc-list-number";
        const nestedNumber = parts.number.replace(/\.$/, "");
        number.textContent = `${nestedNumber.includes(".") ? nestedNumber : parts.number} `;
        link.append(number);
      }
      link.append(document.createTextNode(parts.title));
      item.append(link);
      list.append(item);

      return { heading, item, link };
    });

    let frame = 0;
    let activeIndex = -1;

    const findActiveIndex = () => {
      const anchorY = activeAnchorY();

      for (let index = items.length - 1; index >= 0; index -= 1) {
        if (headingTop(items[index].heading) <= anchorY) return index;
      }

      return items.findIndex(({ heading }) => {
        const rect = heading.getBoundingClientRect();
        return rect.top < globalThis.innerHeight && rect.bottom >= 0;
      });
    };

    const setActiveIndex = (nextActiveIndex) => {
      if (activeIndex === nextActiveIndex) return;

      activeIndex = nextActiveIndex;
      items.forEach(({ item, link }, index) => {
        const active = index === activeIndex;
        item.classList.toggle("is-active", active);
        if (active) {
          link.setAttribute("aria-current", "location");
        } else {
          link.removeAttribute("aria-current");
        }
      });
    };

    function scheduleActiveUpdate() {
      if (frame) return;

      frame = globalThis.requestAnimationFrame(() => {
        frame = 0;
        setActiveIndex(findActiveIndex());
      });
    }

    scheduleActiveUpdate();
    globalThis.requestAnimationFrame(() => toc.classList.add("is-ready"));
    globalThis.addEventListener("load", scheduleActiveUpdate, {
      passive: true,
    });
    globalThis.addEventListener("resize", scheduleActiveUpdate, {
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

  startWhenReady();
})();
