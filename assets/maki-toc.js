(() => {
  const DESKTOP_QUERY = "(min-width: 960px)";
  const HEADING_SELECTOR =
    "h2, h3, h4, h5, h6, [role='heading'][aria-level]";
  const PANEL_HOVER_WIDTH = 280;
  const RAIL_HOVER_WIDTH = 52;
  const MARKER_HIT_RADIUS = 28;
  const TRACK_MARGIN = 16;

  const getHeadingLevel = (heading) => {
    const ariaLevel = Number(heading.getAttribute("aria-level"));
    if (Number.isInteger(ariaLevel) && ariaLevel > 1) return ariaLevel;

    const match = heading.tagName.match(/^H([2-6])$/);
    return match ? Number(match[1]) : 6;
  };

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

  const documentHeight = () => {
    const { body, documentElement } = document;

    return Math.max(
      body.offsetHeight,
      body.scrollHeight,
      documentElement.offsetHeight,
      documentElement.scrollHeight,
      globalThis.innerHeight,
    );
  };

  const markerY = (heading) => {
    const trackHeight = globalThis.innerHeight - TRACK_MARGIN * 2;
    const ratio = Math.min(Math.max(headingTop(heading) / documentHeight(), 0), 1);

    return TRACK_MARGIN + ratio * trackHeight;
  };

  const start = () => {
    if (!globalThis.matchMedia(DESKTOP_QUERY).matches) return;

    const headings = collectHeadings();
    if (!headings.length) return;

    const toc = document.createElement("nav");
    toc.className = "maki-toc";
    toc.setAttribute("aria-label", "Section map");

    const markers = document.createElement("div");
    markers.className = "maki-toc-markers";
    const labels = document.createElement("div");
    labels.className = "maki-toc-labels";
    toc.append(markers, labels);
    document.body.append(toc);

    const labelTexts = numberedLabels(headings);
    const items = headings.map((heading, index) => {
      const labelText = labelTexts[index];
      const marker = document.createElement("button");
      marker.type = "button";
      marker.className = "maki-toc-marker";
      marker.setAttribute("aria-label", labelText);
      marker.style.setProperty(
        "--maki-toc-delay",
        `${Math.min(index * 14, 180)}ms`,
      );

      const label = document.createElement("button");
      label.type = "button";
      label.className = "maki-toc-label";
      label.textContent = labelText;
      label.setAttribute("aria-hidden", "true");

      const navigate = () => {
        heading.scrollIntoView({ block: "start" });
        if (heading.id) {
          globalThis.history.replaceState(
            null,
            "",
            `${globalThis.location.pathname}${globalThis.location.search}#${encodeURIComponent(heading.id)}`,
          );
        }
      };

      marker.addEventListener("click", navigate);
      label.addEventListener("click", navigate);
      marker.addEventListener("mouseenter", () => showSingle(index));
      label.addEventListener("mouseenter", () => showAll(index));

      markers.append(marker);
      labels.append(label);

      return { heading, label, marker, y: 0 };
    });

    let frame = 0;
    let mode = "hidden";
    let activeIndex = -1;

    const setMode = (nextMode, nextActiveIndex = -1) => {
      if (mode === nextMode && activeIndex === nextActiveIndex) return;

      mode = nextMode;
      activeIndex = nextActiveIndex;

      items.forEach((item, index) => {
        const showLabel =
          mode === "all" || (mode === "single" && index === activeIndex);
        item.label.classList.toggle("is-visible", showLabel);
        item.label.setAttribute("aria-hidden", showLabel ? "false" : "true");
        item.marker.classList.toggle("is-active", index === activeIndex);
      });
    };

    function showSingle(index) {
      setMode("single", index);
    }

    function showAll(index) {
      setMode("all", index);
    }

    const hideLabels = () => {
      setMode("hidden");
    };

    const nearestItem = (clientY) => {
      let nearest = null;

      items.forEach((item, index) => {
        const distance = Math.abs(item.y - clientY);
        if (!nearest || distance < nearest.distance) {
          nearest = { distance, index };
        }
      });

      return nearest && nearest.distance <= MARKER_HIT_RADIUS ? nearest : null;
    };

    const layout = () => {
      items.forEach((item) => {
        item.y = markerY(item.heading);
        const top = `${item.y}px`;
        item.marker.style.top = top;
        item.label.style.top = top;
      });
    };

    const scheduleLayout = () => {
      if (frame) return;

      frame = globalThis.requestAnimationFrame(() => {
        frame = 0;
        layout();
      });
    };

    const handlePointerMove = (event) => {
      const panelLeft = globalThis.innerWidth - PANEL_HOVER_WIDTH;
      if (event.clientX < panelLeft) {
        hideLabels();
        return;
      }

      const nearest = nearestItem(event.clientY);
      if (!nearest) {
        hideLabels();
        return;
      }

      const railLeft = globalThis.innerWidth - RAIL_HOVER_WIDTH;
      if (event.clientX >= railLeft) {
        showSingle(nearest.index);
      } else {
        showAll(nearest.index);
      }
    };

    layout();
    globalThis.requestAnimationFrame(() => toc.classList.add("is-ready"));
    globalThis.addEventListener("load", scheduleLayout, { passive: true });
    globalThis.addEventListener("resize", scheduleLayout, { passive: true });
    globalThis.addEventListener("mousemove", handlePointerMove, {
      passive: true,
    });
    document.addEventListener("mouseleave", hideLabels);
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start, { once: true });
  } else {
    start();
  }
})();
