const assert = require("node:assert/strict");
const fs = require("node:fs");
const test = require("node:test");
const vm = require("node:vm");

const loadExternalLinksTestExports = () => {
  let exports;
  const context = {
    URL,
    __makiExternalLinksUnitTestExports: (api) => {
      exports = api;
    },
  };

  vm.createContext(context);
  vm.runInContext(
    fs.readFileSync("assets/maki-external-links.js", "utf8"),
    context,
    { filename: "assets/maki-external-links.js" },
  );

  assert.ok(exports, "maki-external-links.js should expose test exports");
  return exports;
};

const { faviconUrlForHref } = loadExternalLinksTestExports();

test("uses the external origin favicon regardless of the link path", () => {
  assert.equal(
    faviconUrlForHref(
      "https://example.com/docs/page?query=yes#section",
      "http://localhost:4000/note",
    ),
    "https://example.com/favicon.ico",
  );
});

test("keeps protocol-relative hosts and explicit ports", () => {
  assert.equal(
    faviconUrlForHref("//example.com:8443/page", "https://localhost/note"),
    "https://example.com:8443/favicon.ico",
  );
});

test("does not request favicons for non-http external links", () => {
  assert.equal(
    faviconUrlForHref("mailto:hello@example.com", "http://localhost/note"),
    null,
  );
  assert.equal(faviconUrlForHref("broken url", "not a base url"), null);
});
