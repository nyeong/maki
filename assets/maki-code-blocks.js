(() => {
  const CODE_BLOCK_SELECTOR = "[data-maki-code-block]";
  const CODE_ACTIONS_SELECTOR = "[data-maki-code-actions]";
  const ENHANCED_ATTRIBUTE = "data-maki-code-enhanced";
  const WRAPPED_CLASS = "is-wrapped";
  const COPY_FEEDBACK_MS = 2000;
  const COPY_FEEDBACK_MESSAGES = Object.freeze({
    error: "Copy failed",
    idle: "Copy code",
    success: "Copied",
  });
  const COPY_ICON = `
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none"
      stroke="currentColor" stroke-width="1.8" stroke-linecap="round"
      stroke-linejoin="round" aria-hidden="true" focusable="false">
      <rect x="8" y="8" width="11" height="11" rx="2"></rect>
      <path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"></path>
    </svg>`;
  const WRAP_ICON = `
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none"
      stroke="currentColor" stroke-width="1.8" stroke-linecap="round"
      stroke-linejoin="round" aria-hidden="true" focusable="false">
      <path d="M4 6h16M4 11h12a4 4 0 0 1 0 8h-3"></path>
      <path d="m15 16-3 3 3 3"></path>
    </svg>`;
  const LANGUAGE_ALIASES = Object.freeze({
    "c++": "cpp",
    cs: "csharp",
    golang: "go",
    htm: "xml",
    html: "xml",
    js: "javascript",
    jsx: "javascript",
    md: "markdown",
    "no-highlight": "plaintext",
    nohighlight: "plaintext",
    plain: "plaintext",
    py: "python",
    rb: "ruby",
    rs: "rust",
    sh: "bash",
    shell: "bash",
    text: "plaintext",
    ts: "typescript",
    tsx: "typescript",
    txt: "plaintext",
    xhtml: "xml",
    yml: "yaml",
    zsh: "bash",
  });

  const hasOwn = (object, key) =>
    Object.prototype.hasOwnProperty.call(object, key);

  const normalizeLanguage = (rawLanguage) => {
    if (typeof rawLanguage !== "string") return null;

    let language = rawLanguage.trim().toLowerCase();
    if (language.startsWith("language-")) {
      language = language.slice("language-".length);
    } else if (language.startsWith("lang-")) {
      language = language.slice("lang-".length);
    }

    if (
      !language ||
      language.length > 64 ||
      !/^[a-z0-9][a-z0-9_+#.-]*$/.test(language)
    ) {
      return null;
    }

    return hasOwn(LANGUAGE_ALIASES, language)
      ? LANGUAGE_ALIASES[language]
      : language;
  };

  const makiLanguage = () => ({
    contains: [
      {
        scope: "meta",
        match: /^--[\^v][ \t]+[^:\r\n]+:/,
      },
      {
        scope: "meta",
        match: /^-{3,}(?:[ \t]*[A-Za-z0-9_]+(?:[ \t]+[^ \t\r\n]+)*)?[ \t]*$/,
      },
      {
        scope: "section",
        match: /^={1,6}[ \t]+/,
      },
      {
        scope: "bullet",
        match: /^[ \t]*(?:-|[0-9]+\.)[ \t]+/,
      },
      {
        scope: "quote",
        match: /^>[ \t]?/,
      },
      {
        scope: "code",
        match: /^:[ \t]?/,
      },
      {
        scope: "link",
        match: /\[\[[^\]\r\n]+\]\]/,
      },
      {
        scope: "link",
        match: /https?:\/\/[^\s<>\r\n]+/,
      },
      {
        scope: "literal",
        match: /[\[<]\d{4}-(?:\d{2}-\d{2}|W\d{2}(?:-[1-7])?)[^\]>\r\n]*[\]>]/,
      },
      {
        scope: "code",
        begin: /`/,
        end: /`/,
        relevance: 0,
      },
      {
        scope: "strong",
        begin: /\*(?=\S)/,
        end: /\*/,
        relevance: 0,
      },
    ],
    name: "Maki",
  });

  const registerMakiLanguage = (highlighter) => {
    if (
      !highlighter ||
      typeof highlighter.getLanguage !== "function" ||
      typeof highlighter.registerLanguage !== "function"
    ) {
      return false;
    }

    try {
      if (highlighter.getLanguage("maki")) return true;

      highlighter.registerLanguage("maki", makiLanguage);
      return Boolean(highlighter.getLanguage("maki"));
    } catch {
      return false;
    }
  };

  const languageFromClassName = (className) => {
    if (typeof className !== "string") return null;

    const match = className.match(/(?:^|\s)(?:language|lang)-([^\s]+)/i);
    return match ? match[1] : null;
  };

  const languageForCode = (block, code) => {
    const attributeNames = ["data-language", "data-maki-language"];

    for (const attributeName of attributeNames) {
      const value = code.getAttribute(attributeName);
      if (value) return value;
    }

    const classLanguage = languageFromClassName(code.getAttribute("class"));
    if (classLanguage) return classLanguage;

    for (const attributeName of attributeNames) {
      const value = block.getAttribute(attributeName);
      if (value) return value;
    }

    return languageFromClassName(block.getAttribute("class"));
  };

  const highlightSource = (highlighter, source, rawLanguage) => {
    const text = typeof source === "string" ? source : String(source ?? "");
    const language = normalizeLanguage(rawLanguage);
    const unchanged = { highlighted: false, language, value: text };

    if (
      !language ||
      language === "plaintext" ||
      !highlighter ||
      typeof highlighter.getLanguage !== "function" ||
      typeof highlighter.highlight !== "function"
    ) {
      return unchanged;
    }

    try {
      if (!highlighter.getLanguage(language)) return unchanged;

      const result = highlighter.highlight(text, {
        ignoreIllegals: true,
        language,
      });
      if (!result || typeof result.value !== "string") return unchanged;

      return { highlighted: true, language, value: result.value };
    } catch {
      return unchanged;
    }
  };

  const selectedRanges = (documentObject) => {
    if (typeof documentObject.getSelection !== "function") return [];

    try {
      const selection = documentObject.getSelection();
      if (!selection || typeof selection.getRangeAt !== "function") return [];

      const ranges = [];
      for (let index = 0; index < selection.rangeCount; index += 1) {
        const range = selection.getRangeAt(index);
        ranges.push(
          typeof range.cloneRange === "function" ? range.cloneRange() : range,
        );
      }
      return ranges;
    } catch {
      return [];
    }
  };

  const restoreSelection = (documentObject, ranges) => {
    if (!ranges.length || typeof documentObject.getSelection !== "function") {
      return;
    }

    try {
      const selection = documentObject.getSelection();
      selection.removeAllRanges();
      ranges.forEach((range) => selection.addRange(range));
    } catch {
      // Selection restoration is best-effort after the copy has completed.
    }
  };

  const restoreFocus = (element) => {
    if (!element || typeof element.focus !== "function") return;

    try {
      element.focus({ preventScroll: true });
    } catch {
      try {
        element.focus();
      } catch {
        // Focus restoration is best-effort after the copy has completed.
      }
    }
  };

  const legacyCopyText = (source, documentObject) => {
    if (
      !documentObject ||
      !documentObject.body ||
      typeof documentObject.createElement !== "function" ||
      typeof documentObject.execCommand !== "function"
    ) {
      return false;
    }

    const text = typeof source === "string" ? source : String(source ?? "");
    const activeElement = documentObject.activeElement;
    const ranges = selectedRanges(documentObject);
    let textarea;

    try {
      textarea = documentObject.createElement("textarea");
      textarea.value = text;
      textarea.setAttribute("readonly", "");
      textarea.setAttribute("tabindex", "-1");
      textarea.style.height = "1px";
      textarea.style.left = "-9999px";
      textarea.style.opacity = "0";
      textarea.style.position = "fixed";
      textarea.style.top = "0";
      textarea.style.width = "1px";
      documentObject.body.appendChild(textarea);
      textarea.focus();
      textarea.select();
      if (typeof textarea.setSelectionRange === "function") {
        textarea.setSelectionRange(0, textarea.value.length);
      }
      return documentObject.execCommand("copy") === true;
    } catch {
      return false;
    } finally {
      if (textarea && textarea.parentNode) {
        textarea.parentNode.removeChild(textarea);
      }
      restoreSelection(documentObject, ranges);
      restoreFocus(activeElement);
    }
  };

  const copyText = async (source, environment = globalThis) => {
    const navigatorObject = environment && environment.navigator;
    const clipboard = navigatorObject && navigatorObject.clipboard;
    const text = typeof source === "string" ? source : String(source ?? "");

    if (clipboard && typeof clipboard.writeText === "function") {
      try {
        await clipboard.writeText(text);
        return true;
      } catch {
        // A denied or unavailable async clipboard can still use the legacy path.
      }
    }

    return legacyCopyText(text, environment && environment.document);
  };

  const createActionButton = (
    documentObject,
    action,
    labelText,
    iconMarkup,
  ) => {
    const button = documentObject.createElement("button");
    button.type = "button";
    button.className = `maki-code-action maki-code-${action}`;
    button.title = labelText;
    button.setAttribute("aria-label", labelText);
    button.setAttribute("data-maki-code-action", action);

    const icon = documentObject.createElement("span");
    icon.className = "maki-code-action-icon";
    icon.innerHTML = iconMarkup;
    icon.setAttribute("aria-hidden", "true");

    const label = documentObject.createElement("span");
    label.className = "maki-code-action-label";
    label.textContent = labelText;
    button.append(icon, label);

    return { button, label };
  };

  const setWrapped = (block, button, wrapped) => {
    const nextWrapped = Boolean(wrapped);
    block.classList.toggle(WRAPPED_CLASS, nextWrapped);
    button.setAttribute("aria-pressed", nextWrapped ? "true" : "false");
    return nextWrapped;
  };

  const setCopyFeedback = (button, label, status, state) => {
    const message =
      COPY_FEEDBACK_MESSAGES[state] || COPY_FEEDBACK_MESSAGES.idle;

    button.classList.toggle("is-success", state === "success");
    button.classList.toggle("is-error", state === "error");
    button.setAttribute("aria-label", message);
    button.title = message;
    label.textContent = message;
    status.textContent = state === "idle" ? "" : message;
  };

  const enhanceCodeBlock = (block, dependencies = {}) => {
    if (!block || block.hasAttribute(ENHANCED_ATTRIBUTE)) return null;

    const actions = block.querySelector(CODE_ACTIONS_SELECTOR);
    const pre = block.querySelector("pre");
    const code = pre && pre.querySelector("code");
    if (!actions || !code) return null;

    const documentObject =
      dependencies.document || block.ownerDocument || globalThis.document;
    const highlighter = hasOwn(dependencies, "highlighter")
      ? dependencies.highlighter
      : globalThis.hljs;
    const copy =
      dependencies.copy ||
      ((text) =>
        copyText(text, {
          document: documentObject,
          navigator: globalThis.navigator,
        }));
    const schedule = dependencies.setTimeout || globalThis.setTimeout;
    const cancel = dependencies.clearTimeout || globalThis.clearTimeout;
    const originalCode = code.textContent;
    const rawLanguage = languageForCode(block, code);

    registerMakiLanguage(highlighter);
    const highlighted = highlightSource(highlighter, originalCode, rawLanguage);
    if (highlighted.highlighted) {
      code.innerHTML = highlighted.value;
      code.classList.add("hljs");
    }

    const copyControl = createActionButton(
      documentObject,
      "copy",
      "Copy code",
      COPY_ICON,
    );
    const wrapControl = createActionButton(
      documentObject,
      "wrap",
      "Wrap lines",
      WRAP_ICON,
    );
    const status = documentObject.createElement("span");
    status.className = "maki-code-status";
    status.setAttribute("aria-atomic", "true");
    status.setAttribute("aria-live", "polite");
    status.setAttribute("role", "status");

    let copyAttempt = 0;
    let feedbackTimer = null;
    copyControl.button.addEventListener("click", async () => {
      const attempt = ++copyAttempt;
      let copied = false;
      try {
        copied = (await copy(originalCode)) === true;
      } catch {
        copied = false;
      }
      if (attempt !== copyAttempt) return;

      if (feedbackTimer !== null) cancel(feedbackTimer);
      setCopyFeedback(
        copyControl.button,
        copyControl.label,
        status,
        copied ? "success" : "error",
      );
      feedbackTimer = schedule(() => {
        feedbackTimer = null;
        setCopyFeedback(copyControl.button, copyControl.label, status, "idle");
      }, COPY_FEEDBACK_MS);
    });

    setWrapped(block, wrapControl.button, false);
    wrapControl.button.addEventListener("click", () => {
      const wrapped =
        wrapControl.button.getAttribute("aria-pressed") !== "true";
      setWrapped(block, wrapControl.button, wrapped);
    });

    actions.append(copyControl.button, wrapControl.button, status);
    block.setAttribute(ENHANCED_ATTRIBUTE, "");

    return {
      copyButton: copyControl.button,
      originalCode,
      status,
      wrapButton: wrapControl.button,
    };
  };

  const initializeCodeBlocks = (
    documentObject = globalThis.document,
    highlighter = globalThis.hljs,
  ) => {
    if (
      !documentObject ||
      typeof documentObject.querySelectorAll !== "function"
    ) {
      return 0;
    }

    registerMakiLanguage(highlighter);
    let enhancedCount = 0;
    documentObject.querySelectorAll(CODE_BLOCK_SELECTOR).forEach((block) => {
      if (enhanceCodeBlock(block, { document: documentObject, highlighter })) {
        enhancedCount += 1;
      }
    });
    return enhancedCount;
  };

  if (globalThis.__makiCodeBlocksUnitTestExports) {
    globalThis.__makiCodeBlocksUnitTestExports({
      enhanceCodeBlock,
      highlightSource,
      initializeCodeBlocks,
      languageForCode,
      languageFromClassName,
      legacyCopyText,
      makiLanguage,
      normalizeLanguage,
      registerMakiLanguage,
      setCopyFeedback,
      setWrapped,
      copyText,
    });
    return;
  }

  if (document.readyState === "loading") {
    document.addEventListener(
      "DOMContentLoaded",
      () => initializeCodeBlocks(),
      { once: true },
    );
  } else {
    initializeCodeBlocks();
  }
})();
