const assert = require("node:assert/strict");
const fs = require("node:fs");
const test = require("node:test");
const vm = require("node:vm");

const loadSearchTestExports = () => {
  let exports;
  const context = {
    document: { querySelectorAll: () => [] },
    __makiSearchUnitTestExports: (api) => {
      exports = api;
    },
  };

  vm.createContext(context);
  vm.runInContext(fs.readFileSync("assets/maki-search.js", "utf8"), context, {
    filename: "assets/maki-search.js",
  });

  assert.ok(exports, "maki-search.js should expose test exports");
  return exports;
};

const { displayTitle, findMatches } = loadSearchTestExports();

test("returns each target path only once after ranking matches", () => {
  const entries = [
    {
      kind: "note",
      title: "JLPT",
      path: "/notes/JLPT",
      source_path: "notes/JLPT.maki",
    },
    {
      kind: "heading",
      title: "JLPT N2",
      path: "/notes/future#JLPT N2",
      source_path: "notes/future.maki#JLPT N2",
    },
    {
      kind: "file",
      title: "notes/JLPT.maki",
      path: "/notes/JLPT",
      source_path: "notes/JLPT.maki",
    },
  ];

  const matches = findMatches(entries, "JLP");

  assert.deepEqual(
    matches.map((entry) => entry.kind),
    ["note", "heading"],
  );
  assert.equal(matches.filter((entry) => entry.path === "/notes/JLPT").length, 1);
});

test("prefixes heading result titles without changing other titles", () => {
  assert.equal(displayTitle({ kind: "heading", title: "JLPT N2" }), "#JLPT N2");
  assert.equal(displayTitle({ kind: "note", title: "JLPT" }), "JLPT");
});
