const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const test = require("node:test");
const vm = require("node:vm");

const loadCodeBlockTestExports = () => {
  let exports;
  const context = {
    __makiCodeBlocksUnitTestExports: (api) => {
      exports = api;
    },
  };

  vm.createContext(context);
  vm.runInContext(
    fs.readFileSync("assets/vendor/highlight.js/highlight.min.js", "utf8"),
    context,
    { filename: "assets/vendor/highlight.js/highlight.min.js" },
  );
  vm.runInContext(
    fs.readFileSync("assets/maki-code-blocks.js", "utf8"),
    context,
    { filename: "assets/maki-code-blocks.js" },
  );

  assert.ok(exports, "maki-code-blocks.js should expose test exports");
  return { api: exports, highlighter: context.hljs };
};

const { api, highlighter } = loadCodeBlockTestExports();
const {
  copyText,
  enhanceCodeBlock,
  highlightSource,
  initializeCodeBlocks,
  legacyCopyText,
  normalizeLanguage,
  registerMakiLanguage,
} = api;

test("uses the reviewed highlight.js 11.12.0 browser artifact", () => {
  const source = fs.readFileSync("assets/vendor/highlight.js/highlight.min.js");
  const digest = crypto.createHash("sha256").update(source).digest("hex");

  assert.equal(highlighter.versionString, "11.12.0");
  assert.equal(
    digest,
    "8ab71eb09c51f501e5e25157d9cff100e46cc29bcbfc744d0b746d451fca7f53",
  );
});

class FakeClassList {
  constructor() {
    this.values = new Set();
  }

  add(...values) {
    values.forEach((value) => this.values.add(value));
  }

  contains(value) {
    return this.values.has(value);
  }

  toggle(value, force) {
    const enabled =
      force === undefined ? !this.contains(value) : Boolean(force);
    if (enabled) {
      this.values.add(value);
    } else {
      this.values.delete(value);
    }
    return enabled;
  }
}

class FakeElement {
  constructor(tagName, ownerDocument) {
    this.attributes = new Map();
    this.children = [];
    this.classList = new FakeClassList();
    this.className = "";
    this.innerHTMLWrites = 0;
    this.listeners = new Map();
    this.ownerDocument = ownerDocument;
    this.parentNode = null;
    this.queries = new Map();
    this.style = {};
    this.tagName = tagName.toUpperCase();
    this.title = "";
    this.type = "";
    this.value = "";
    this._innerHTML = "";
    this._textContent = "";
  }

  get innerHTML() {
    return this._innerHTML;
  }

  set innerHTML(value) {
    this._innerHTML = value;
    this.innerHTMLWrites += 1;
  }

  get textContent() {
    if (this.children.length) {
      return this.children.map((child) => child.textContent).join("");
    }
    return this._textContent;
  }

  set textContent(value) {
    this.children = [];
    this._textContent = String(value);
  }

  addEventListener(type, listener) {
    const listeners = this.listeners.get(type) || [];
    listeners.push(listener);
    this.listeners.set(type, listeners);
  }

  append(...children) {
    children.forEach((child) => {
      child.parentNode = this;
      this.children.push(child);
    });
  }

  appendChild(child) {
    this.append(child);
    return child;
  }

  async click() {
    const listeners = this.listeners.get("click") || [];
    for (const listener of listeners) {
      await listener({ currentTarget: this, target: this });
    }
  }

  getAttribute(name) {
    return this.attributes.has(name) ? this.attributes.get(name) : null;
  }

  hasAttribute(name) {
    return this.attributes.has(name);
  }

  querySelector(selector) {
    if (this.queries.has(selector)) return this.queries.get(selector);
    if (!selector.startsWith(".")) return null;

    const className = selector.slice(1);
    return (
      this.children.find((child) =>
        child.className.split(/\s+/).includes(className),
      ) || null
    );
  }

  removeChild(child) {
    const index = this.children.indexOf(child);
    if (index >= 0) this.children.splice(index, 1);
    child.parentNode = null;
    return child;
  }

  setAttribute(name, value) {
    this.attributes.set(name, String(value));
  }

  setQuery(selector, result) {
    this.queries.set(selector, result);
  }
}

class FakeDocument {
  constructor() {
    this.activeElement = null;
    this.blocks = [];
    this.body = new FakeElement("body", this);
    this.execCommandResult = false;
    this.execCommands = [];
    this.textareas = [];
  }

  createElement(tagName) {
    const element = new FakeElement(tagName, this);
    if (tagName === "textarea") {
      element.focus = () => {
        element.focused = true;
      };
      element.select = () => {
        element.selected = true;
      };
      element.setSelectionRange = (start, end) => {
        element.selectionRange = [start, end];
      };
      this.textareas.push(element);
    }
    return element;
  }

  execCommand(command) {
    this.execCommands.push(command);
    return this.execCommandResult;
  }

  querySelectorAll(selector) {
    assert.equal(selector, "[data-maki-code-block]");
    return this.blocks;
  }
}

const makeCodeBlock = (documentObject, language, source = "const n = 1;") => {
  const block = new FakeElement("div", documentObject);
  const actions = new FakeElement("div", documentObject);
  const pre = new FakeElement("pre", documentObject);
  const code = new FakeElement("code", documentObject);
  code.textContent = source;
  if (language !== null) code.setAttribute("data-language", language);
  block.setQuery("[data-maki-code-actions]", actions);
  block.setQuery("pre", pre);
  pre.setQuery("code", code);
  return { actions, block, code, pre };
};

const actionButton = (actions, action) =>
  actions.children.find(
    (child) => child.getAttribute("data-maki-code-action") === action,
  );

const assertIconOnlyButton = (button, accessibleName) => {
  assert.equal(button.tagName, "BUTTON");
  assert.equal(button.type, "button");
  assert.equal(button.children.length, 1);
  assert.equal(button.children[0].className, "maki-code-action-icon");
  assert.equal(button.children[0].getAttribute("aria-hidden"), "true");
  assert.equal(button.textContent.trim(), "");
  assert.equal(button.getAttribute("aria-label"), accessibleName);
  assert.equal(button.title, accessibleName);
};

test("normalizes common language aliases without accepting malformed names", () => {
  assert.equal(normalizeLanguage(" HTML "), "xml");
  assert.equal(normalizeLanguage("language-js"), "javascript");
  assert.equal(normalizeLanguage("lang-ts"), "typescript");
  assert.equal(normalizeLanguage("sh"), "bash");
  assert.equal(normalizeLanguage("shell"), "bash");
  assert.equal(normalizeLanguage("zsh"), "bash");
  assert.equal(normalizeLanguage("rs"), "rust");
  assert.equal(normalizeLanguage("txt"), "plaintext");
  assert.equal(normalizeLanguage("plain"), "plaintext");
  assert.equal(normalizeLanguage("plaintext"), "plaintext");
  assert.equal(normalizeLanguage("custom-lang"), "custom-lang");
  assert.equal(normalizeLanguage("not a language"), null);
  assert.equal(normalizeLanguage(""), null);
  assert.equal(normalizeLanguage(null), null);
});

test("registers Maki alongside the baseline bundled languages", () => {
  assert.equal(registerMakiLanguage(highlighter), true);
  assert.ok(highlighter.getLanguage("maki"));
  assert.equal(highlighter.getLanguage("mk").name, "Makefile");

  const examples = [
    ["html", "<main>hello</main>"],
    ["javascript", "const answer = 42;"],
    ["bash", 'echo "$HOME"'],
    ["rust", 'fn main() { println!("hello"); }'],
    ["python", "def greet(name):\n    return name"],
    ["json", '{"ready": true}'],
    ["css", ".note { color: rebeccapurple; }"],
    ["toml", 'title = "Maki"'],
    [
      "maki",
      "--^ status: stable\n= Heading\n---code maki\nSee [[another-note]].\n---",
    ],
  ];

  examples.forEach(([language, source]) => {
    const result = highlightSource(highlighter, source, language);
    assert.equal(result.highlighted, true, `${language} should be highlighted`);
    assert.match(result.value, /hljs-/);
  });

  const escaped = highlightSource(
    highlighter,
    "<script>alert(1)</script>",
    "maki",
  );
  assert.doesNotMatch(escaped.value, /<script>/);
  assert.match(escaped.value, /&lt;script&gt;/);

  const container = highlightSource(
    highlighter,
    "----- code rust\nfn main() {}\n-----",
    "maki",
  );
  assert.match(container.value, /hljs-meta/);
});

test("unknown, absent, and plaintext languages remain exactly lossless", () => {
  let highlightCalls = 0;
  const fakeHighlighter = {
    getLanguage: (language) => language === "plaintext",
    highlight: () => {
      highlightCalls += 1;
      throw new Error("highlight should not run");
    },
    registerLanguage: () => {},
  };
  const source = '<b data-value="&">unchanged</b>\n';

  ["unknown-language", "plain", null].forEach((language) => {
    const result = highlightSource(fakeHighlighter, source, language);
    assert.equal(result.highlighted, false);
    assert.equal(result.value, source);
  });
  assert.equal(highlightCalls, 0);
});

test("highlight failures return the untouched source", () => {
  const source = "fn main() {}";
  const result = highlightSource(
    {
      getLanguage: () => true,
      highlight: () => {
        throw new Error("broken grammar");
      },
    },
    source,
    "rust",
  );

  assert.deepEqual(
    { highlighted: result.highlighted, value: result.value },
    { highlighted: false, value: source },
  );

  const lookupFailure = highlightSource(
    {
      getLanguage: () => {
        throw new Error("broken registry");
      },
      highlight: () => "unreachable",
    },
    source,
    "rust",
  );
  assert.equal(lookupFailure.highlighted, false);
  assert.equal(lookupFailure.value, source);
});

test("uses the async clipboard when it succeeds", async () => {
  const writes = [];
  const documentObject = new FakeDocument();
  documentObject.execCommandResult = true;

  const copied = await copyText("exact\ncode", {
    document: documentObject,
    navigator: {
      clipboard: {
        writeText: async (text) => writes.push(text),
      },
    },
  });

  assert.equal(copied, true);
  assert.deepEqual(writes, ["exact\ncode"]);
  assert.deepEqual(documentObject.execCommands, []);
});

test("falls back to a selected temporary textarea and always cleans it up", async () => {
  const documentObject = new FakeDocument();
  documentObject.execCommandResult = true;

  const copied = await copyText("fallback <code>", {
    document: documentObject,
    navigator: {
      clipboard: {
        writeText: async () => {
          throw new Error("permission denied");
        },
      },
    },
  });

  assert.equal(copied, true);
  assert.deepEqual(documentObject.execCommands, ["copy"]);
  assert.equal(documentObject.body.children.length, 0);
  assert.equal(documentObject.textareas[0].value, "fallback <code>");
  assert.equal(documentObject.textareas[0].selected, true);
  assert.deepEqual(documentObject.textareas[0].selectionRange, [0, 15]);
  assert.equal(documentObject.textareas[0].getAttribute("tabindex"), "-1");
  assert.equal(documentObject.textareas[0].hasAttribute("aria-hidden"), false);
});

test("reports failure when neither copy path is available", async () => {
  assert.equal(await copyText("nope", { navigator: {} }), false);

  const documentObject = new FakeDocument();
  documentObject.execCommandResult = false;
  assert.equal(legacyCopyText("nope", documentObject), false);
  assert.equal(documentObject.body.children.length, 0);
});

test("enhancement highlights once but always copies the original source", async () => {
  const documentObject = new FakeDocument();
  const source = "const answer = 42;\n";
  const fixture = makeCodeBlock(documentObject, "js", source);
  const copied = [];
  const timers = [];
  const dependencies = {
    clearTimeout: () => {},
    copy: async (text) => {
      copied.push(text);
      return true;
    },
    document: documentObject,
    highlighter,
    setTimeout: (callback, delay) => {
      timers.push({ callback, delay });
      return timers.length;
    },
  };

  const controls = enhanceCodeBlock(fixture.block, dependencies);
  assert.ok(controls);
  assert.equal(fixture.code.innerHTMLWrites, 1);
  assert.match(fixture.code.innerHTML, /hljs-keyword/);
  assert.equal(fixture.code.classList.contains("hljs"), true);
  assert.equal(fixture.actions.children.length, 3);
  assertIconOnlyButton(controls.copyButton, "Copy code");
  assertIconOnlyButton(controls.wrapButton, "Wrap lines");
  const idleIcon = controls.copyButton.children[0].innerHTML;
  assert.match(idleIcon, /data-icon="copy"/);

  await controls.copyButton.click();
  assert.deepEqual(copied, [source]);
  assert.equal(controls.copyButton.getAttribute("aria-label"), "Copied");
  assert.equal(controls.copyButton.title, "Copied");
  assert.equal(controls.status.textContent, "Copied");
  assert.equal(controls.copyButton.classList.contains("is-success"), true);
  assert.equal(controls.copyButton.textContent.trim(), "");
  assert.equal(controls.copyButton.children.length, 1);
  assert.notEqual(controls.copyButton.children[0].innerHTML, idleIcon);
  assert.match(controls.copyButton.children[0].innerHTML, /data-icon="check"/);
  assert.equal(timers[0].delay, 2000);

  timers[0].callback();
  assert.equal(controls.copyButton.getAttribute("aria-label"), "Copy code");
  assert.equal(controls.copyButton.title, "Copy code");
  assert.equal(controls.status.textContent, "");
  assert.equal(controls.copyButton.classList.contains("is-success"), false);
  assert.equal(controls.copyButton.children[0].innerHTML, idleIcon);
  assertIconOnlyButton(controls.copyButton, "Copy code");

  assert.equal(enhanceCodeBlock(fixture.block, dependencies), null);
  assert.equal(fixture.code.innerHTMLWrites, 1);
  assert.equal(fixture.actions.children.length, 3);
});

test("copy failures are announced and reset", async () => {
  const documentObject = new FakeDocument();
  const fixture = makeCodeBlock(documentObject, null, "untyped source");
  const timers = [];
  const controls = enhanceCodeBlock(fixture.block, {
    copy: async () => false,
    document: documentObject,
    highlighter,
    setTimeout: (callback) => {
      timers.push(callback);
      return timers.length;
    },
  });
  const idleIcon = controls.copyButton.children[0].innerHTML;

  await controls.copyButton.click();
  assert.equal(controls.copyButton.getAttribute("aria-label"), "Copy failed");
  assert.equal(controls.copyButton.title, "Copy failed");
  assert.equal(controls.status.textContent, "Copy failed");
  assert.equal(controls.copyButton.classList.contains("is-error"), true);
  assert.equal(controls.copyButton.textContent.trim(), "");
  assert.equal(controls.copyButton.children.length, 1);
  assert.notEqual(controls.copyButton.children[0].innerHTML, idleIcon);
  assert.match(controls.copyButton.children[0].innerHTML, /data-icon="error"/);
  timers[0]();
  assert.equal(controls.copyButton.getAttribute("aria-label"), "Copy code");
  assert.equal(controls.copyButton.title, "Copy code");
  assert.equal(controls.copyButton.classList.contains("is-error"), false);
  assert.equal(controls.copyButton.children[0].innerHTML, idleIcon);
});

test("plain and unsupported blocks are enhanced without code mutation", () => {
  const documentObject = new FakeDocument();
  const source = "<tag>& untouched\n";
  const fixtures = [
    makeCodeBlock(documentObject, "txt", source),
    makeCodeBlock(documentObject, "made-up", source),
    makeCodeBlock(documentObject, null, source),
  ];

  fixtures.forEach((fixture) => {
    assert.ok(
      enhanceCodeBlock(fixture.block, {
        copy: async () => true,
        document: documentObject,
        highlighter,
        setTimeout: () => 1,
      }),
    );
    assert.equal(fixture.code.textContent, source);
    assert.equal(fixture.code.innerHTMLWrites, 0);
    assert.equal(fixture.code.classList.contains("hljs"), false);
  });
});

test("wrap state is local to each block and initialization is idempotent", async () => {
  const documentObject = new FakeDocument();
  const first = makeCodeBlock(documentObject, "rust", "fn first() {}");
  const second = makeCodeBlock(documentObject, "rust", "fn second() {}");
  documentObject.blocks = [first.block, second.block];

  assert.equal(initializeCodeBlocks(documentObject, highlighter), 2);
  assert.equal(initializeCodeBlocks(documentObject, highlighter), 0);

  const firstWrap = actionButton(first.actions, "wrap");
  const secondWrap = actionButton(second.actions, "wrap");
  assert.equal(firstWrap.getAttribute("aria-pressed"), "false");
  assert.equal(secondWrap.getAttribute("aria-pressed"), "false");

  await firstWrap.click();
  assert.equal(first.block.classList.contains("is-wrapped"), true);
  assert.equal(firstWrap.getAttribute("aria-pressed"), "true");
  assert.equal(second.block.classList.contains("is-wrapped"), false);
  assert.equal(secondWrap.getAttribute("aria-pressed"), "false");

  await firstWrap.click();
  assert.equal(first.block.classList.contains("is-wrapped"), false);
  assert.equal(firstWrap.getAttribute("aria-pressed"), "false");
  assert.equal(first.actions.children.length, 3);
  assert.equal(second.actions.children.length, 3);
});
