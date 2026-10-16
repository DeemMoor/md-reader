// Tests for the text part of the live diff engine (assets/diff.js).
// Run with: node --test tests/diff.test.js
const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");

global.window = {};
global.document = {};
require(path.join(__dirname, "..", "assets", "diff.js"));
const { myers, lineStats, wordSegments, lineSegments } = window.mdrDiff;

// Deterministic PRNG so failures are reproducible.
function rng(seed) {
  return () => {
    seed = (seed * 1103515245 + 12345) & 0x7fffffff;
    return seed / 0x80000000;
  };
}

function lcs(a, b) {
  const d = Array.from({ length: a.length + 1 }, () => new Array(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--)
    for (let j = b.length - 1; j >= 0; j--)
      d[i][j] = a[i] === b[j] ? d[i + 1][j + 1] + 1 : Math.max(d[i + 1][j], d[i][j + 1]);
  return d[0][0];
}

function joined(segs, op) {
  return segs.filter((s) => s.op !== op).map((s) => s.text).join("");
}

test("myers produces a valid and minimal edit script", () => {
  const random = rng(42);
  const seq = () => Array.from({ length: Math.floor(random() * 12) }, () => "abc"[Math.floor(random() * 3)]);
  for (let t = 0; t < 5000; t++) {
    const a = seq();
    const b = seq();
    const ops = myers(a, b);
    assert.deepEqual(ops.filter((o) => o[0] !== "+").map((o) => a[o[1]]), a);
    assert.deepEqual(ops.filter((o) => o[0] !== "-").map((o) => b[o[2]]), b);
    for (const o of ops) if (o[0] === "=") assert.equal(a[o[1]], b[o[2]]);
    const edits = ops.filter((o) => o[0] !== "=").length;
    assert.equal(edits, a.length + b.length - 2 * lcs(a, b), JSON.stringify({ a, b }));
  }
});

test("myers gives up on too many edits", () => {
  const a = Array.from({ length: 200 }, (_, i) => "a" + i);
  const b = Array.from({ length: 200 }, (_, i) => "b" + i);
  assert.equal(myers(a, b, 50), null);
});

test("lineStats counts added and removed lines", () => {
  assert.deepEqual(lineStats("a\nb\nc\n", "a\nx\nc\nd\n"), { add: 2, del: 1 });
  assert.deepEqual(lineStats("same\n", "same\n"), { add: 0, del: 0 });
});

test("lineStats stays fast on a large document", () => {
  const big = Array.from({ length: 50000 }, (_, i) => "line " + i);
  const edited = big.slice();
  edited.splice(25000, 3, "x", "y");
  const t0 = Date.now();
  assert.deepEqual(lineStats(big.join("\n"), edited.join("\n")), { add: 2, del: 3 });
  assert.ok(Date.now() - t0 < 2000);
});

test("wordSegments marks the changed words only", () => {
  const segs = wordSegments("Этап 3 почти готов, осталось два пункта", "Этап 4 полностью готов, осталось два пункта");
  assert.deepEqual(segs, [
    { op: "=", text: "Этап " },
    { op: "-", text: "3 почти" },
    { op: "+", text: "4 полностью" },
    { op: "=", text: " готов, осталось два пункта" },
  ]);
});

test("wordSegments refuses unrelated text unless forced", () => {
  assert.equal(wordSegments("one two three", "completely different words here"), null);
  const forced = wordSegments("one two three", "completely different words here", true);
  assert.equal(joined(forced, "+"), "one two three");
  assert.equal(joined(forced, "-"), "completely different words here");
});

test("lineSegments reconstructs both versions", () => {
  const oldText = "a\nfoo bar baz\nc\n";
  const newText = "a\nfoo qux baz\nc\nd\n";
  const segs = lineSegments(oldText, newText);
  assert.equal(joined(segs, "+"), oldText);
  assert.equal(joined(segs, "-"), newText);
  assert.deepEqual(segs.filter((s) => s.op !== "="), [
    { op: "-", text: "bar" },
    { op: "+", text: "qux" },
    { op: "+", text: "d\n" },
  ]);
});
