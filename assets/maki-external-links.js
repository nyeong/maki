(() => {
  const EXTERNAL_LINK_SELECTOR = "a.external-link";
  const FAVICON_CLASS = "maki-external-link-favicon";
  const HAS_FAVICON_CLASS = "maki-external-link-has-favicon";

  const faviconUrlForHref = (href, baseUrl) => {
    if (!href) return null;

    try {
      const target = new URL(href, baseUrl);
      if (target.protocol !== "http:" && target.protocol !== "https:") {
        return null;
      }

      return new URL("/favicon.ico", target.origin).href;
    } catch {
      return null;
    }
  };

  const decorateLink = (link) => {
    if (link.querySelector(`:scope > .${FAVICON_CLASS}`)) return;

    const faviconUrl = faviconUrlForHref(
      link.getAttribute("href"),
      document.baseURI,
    );
    if (!faviconUrl) return;

    const favicon = document.createElement("img");
    favicon.className = FAVICON_CLASS;
    favicon.alt = "";
    favicon.decoding = "async";
    favicon.referrerPolicy = "no-referrer";
    favicon.setAttribute("aria-hidden", "true");
    favicon.addEventListener(
      "load",
      () => link.classList.add(HAS_FAVICON_CLASS),
      { once: true },
    );
    favicon.addEventListener("error", () => favicon.remove(), { once: true });
    favicon.src = faviconUrl;
    link.prepend(favicon);
  };

  const start = () => {
    document.querySelectorAll(EXTERNAL_LINK_SELECTOR).forEach(decorateLink);
  };

  if (globalThis.__makiExternalLinksUnitTestExports) {
    globalThis.__makiExternalLinksUnitTestExports({ faviconUrlForHref });
    return;
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start, { once: true });
  } else {
    start();
  }
})();
