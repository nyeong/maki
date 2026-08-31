const assert = require("node:assert/strict");
const fs = require("node:fs");
const test = require("node:test");
const vm = require("node:vm");

const loadTocTestExports = (testDocument) => {
  let exports;
  const context = {
    __makiTocUnitTestExports: (api) => {
      exports = api;
    },
  };
  if (testDocument) context.document = testDocument;

  vm.createContext(context);
  vm.runInContext(fs.readFileSync("assets/maki-toc.js", "utf8"), context, {
    filename: "assets/maki-toc.js",
  });

  assert.ok(exports, "maki-toc.js should expose test exports");
  return exports;
};

const { overlappedMarkerIndexes } = loadTocTestExports();

const hiddenIndexes = (boxes) =>
  Array.from(overlappedMarkerIndexes(boxes)).sort((left, right) => left - right);

test("overlapping markers keep the earlier heading at the same level", () => {
  assert.deepEqual(
    hiddenIndexes([
      { bottom: 20, index: 0, level: 1, top: 0 },
      { bottom: 21, index: 1, level: 1, top: 1 },
    ]),
    [1],
  );
});

test("overlapping markers keep the higher section level first", () => {
  assert.deepEqual(
    hiddenIndexes([
      { bottom: 20, index: 0, level: 2, top: 0 },
      { bottom: 21, index: 1, level: 1, top: 1 },
    ]),
    [0],
  );
});

test("non-overlapping markers remain visible", () => {
  assert.deepEqual(
    hiddenIndexes([
      { bottom: 10, index: 0, level: 1, top: 0 },
      { bottom: 30, index: 1, level: 1, top: 20 },
    ]),
    [],
  );
});

const sectionMapInsertion = ({
  breadcrumb = false,
  title = false,
  nav = false,
}) => {
  let insertion;
  const insertionAnchor = (name) => ({
    insertAdjacentElement: (position) => {
      insertion = { anchor: name, position };
    },
  });
  const anchors = {
    breadcrumb: insertionAnchor("breadcrumb"),
    nav: insertionAnchor("nav"),
    title: insertionAnchor("title"),
  };
  const testDocument = {
    body: {
      prepend: () => {
        insertion = { anchor: "body", position: "prepend" };
      },
    },
    querySelector: (selector) => {
      if (selector === "body > .maki-document-breadcrumb") {
        return breadcrumb ? anchors.breadcrumb : null;
      }
      if (selector === "body > h1") return title ? anchors.title : null;
      if (selector === "body > .maki-nav") return nav ? anchors.nav : null;
      return null;
    },
  };
  const { insertToc } = loadTocTestExports(testDocument);
  insertToc({});
  return insertion;
};

test("the section map follows the first available page landmark", () => {
  assert.deepEqual(
    sectionMapInsertion({ breadcrumb: true, title: true, nav: true }),
    {
      anchor: "breadcrumb",
      position: "afterend",
    },
  );
  assert.deepEqual(sectionMapInsertion({ title: true, nav: true }), {
    anchor: "title",
    position: "afterend",
  });
  assert.deepEqual(sectionMapInsertion({ nav: true }), {
    anchor: "nav",
    position: "afterend",
  });
  assert.deepEqual(sectionMapInsertion({}), {
    anchor: "body",
    position: "prepend",
  });
});
