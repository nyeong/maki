const assert = require("node:assert/strict");
const fs = require("node:fs");
const test = require("node:test");
const vm = require("node:vm");

const loadTocTestExports = () => {
  let exports;
  const context = {
    __makiTocUnitTestExports: (api) => {
      exports = api;
    },
  };

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
