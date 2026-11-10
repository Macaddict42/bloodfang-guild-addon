const assert = require("assert");
const { contentFitSize, fitChanged } = require("./window-fit.js");

const limits = { minWidth: 400, minHeight: 240, maxWidth: 1280, maxHeight: 800 };

const compact = contentFitSize({ width: 436, height: 548 }, limits);
assert.deepStrictEqual({ width: compact.width, height: compact.height, clipped: compact.clipped }, {
  width: 436,
  height: 548,
  clipped: false,
});

const huge = contentFitSize({ width: 933, height: 967 }, limits);
assert.strictEqual(huge.width, 933);
assert.strictEqual(huge.height, 800);
assert.strictEqual(huge.clipped, true);
assert.strictEqual(fitChanged(huge, huge), false);
assert.strictEqual(fitChanged(null, compact), true);
assert.strictEqual(fitChanged(compact, { width: 436, height: 549, clipped: false }), true);

const sameText = contentFitSize({ width: 435.1, height: 547.2 }, limits);
assert.strictEqual(fitChanged(compact, sameText), false);

console.log("window-fit.test.js ok");
