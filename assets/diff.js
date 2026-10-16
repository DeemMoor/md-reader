// Live diff of a document against the version the tab first showed.
// Blocks are matched as a whole; inside a changed block the words are compared.
(() => {
  "use strict";

  const MAX_EDITS = 3000;

  // Myers O(ND) diff. Returns [[op, aIndex, bIndex], ...] with op "=", "-" or "+",
  // or null when the inputs differ by more than `maxEdits`.
  function myers(a, b, maxEdits = MAX_EDITS) {
    let start = 0;
    while (start < a.length && start < b.length && a[start] === b[start]) start++;
    let endA = a.length;
    let endB = b.length;
    while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
      endA--;
      endB--;
    }
    const n = endA - start;
    const m = endB - start;
    const ops = [];
    for (let i = 0; i < start; i++) ops.push(["=", i, i]);

    const middle = [];
    if (n === 0) {
      for (let j = 0; j < m; j++) middle.push(["+", -1, start + j]);
    } else if (m === 0) {
      for (let i = 0; i < n; i++) middle.push(["-", start + i, -1]);
    } else {
      const max = Math.min(n + m, maxEdits);
      const offset = max + 1;
      let v = new Int32Array(2 * max + 3);
      const trace = [];
      let found = -1;
      for (let d = 0; d <= max && found < 0; d++) {
        // Only the diagonals step d can read: memory grows as D², not D·(N+M).
        trace.push({ lo: offset - d - 1, v: v.slice(offset - d - 1, offset + d + 2) });
        for (let k = -d; k <= d; k += 2) {
          let x =
            k === -d || (k !== d && v[offset + k - 1] < v[offset + k + 1])
              ? v[offset + k + 1]
              : v[offset + k - 1] + 1;
          let y = x - k;
          while (x < n && y < m && a[start + x] === b[start + y]) {
            x++;
            y++;
          }
          v[offset + k] = x;
          if (x >= n && y >= m) {
            found = d;
            break;
          }
        }
      }
      if (found < 0) return null;
      let x = n;
      let y = m;
      for (let d = found; d > 0; d--) {
        const t = trace[d];
        const at = (k) => t.v[offset + k - t.lo];
        const k = x - y;
        const prevK = k === -d || (k !== d && at(k - 1) < at(k + 1)) ? k + 1 : k - 1;
        const prevX = at(prevK);
        const prevY = prevX - prevK;
        while (x > prevX && y > prevY) middle.push(["=", start + --x, start + --y]);
        if (x === prevX) middle.push(["+", -1, start + --y]);
        else middle.push(["-", start + --x, -1]);
      }
      while (x > 0 && y > 0) middle.push(["=", start + --x, start + --y]);
      middle.reverse();
    }
    ops.push(...middle);
    for (let i = 0; i < a.length - endA; i++) ops.push(["=", endA + i, endB + i]);
    return ops;
  }

  // --- line stats (the "+3 −5" counter) ------------------------------------------

  function splitLines(text) {
    const lines = text.replace(/\r\n?/g, "\n").split("\n");
    if (lines.length && lines[lines.length - 1] === "") lines.pop();
    return lines;
  }

  function lineStats(oldText, newText) {
    const a = splitLines(oldText);
    const b = splitLines(newText);
    const ops = myers(a, b, 4000);
    if (!ops) return { add: b.length, del: a.length };
    let add = 0;
    let del = 0;
    for (const [op] of ops) {
      if (op === "+") add++;
      else if (op === "-") del++;
    }
    return { add, del };
  }

  // --- words ------------------------------------------------------------------------

  const TOKEN = /\s+|[\p{L}\p{N}_]+|[^\s\p{L}\p{N}_]/gu;
  const tokenize = (text) => text.match(TOKEN) || [];
  const isBlank = (s) => !/\S/.test(s);
  const visibleLength = (s) => s.replace(/\s+/g, "").length;

  // Word diff as segments [{op, text}], cleaned up so that a replaced phrase shows as
  // one deleted run followed by one inserted run. Null when the texts barely match.
  function wordSegments(oldText, newText, force = false) {
    const a = tokenize(oldText);
    const b = tokenize(newText);
    const ops = myers(a, b);
    if (!ops) return null;

    let segs = [];
    let same = 0;
    for (const [op, i, j] of ops) {
      const text = op === "+" ? b[j] : a[i];
      if (op === "=") same += visibleLength(text);
      const last = segs[segs.length - 1];
      if (last && last.op === op) last.text += text;
      else segs.push({ op, text });
    }
    const total = Math.max(visibleLength(oldText), visibleLength(newText));
    if (!force && total > 0 && same / total < 0.4) return null;

    // Glue changes separated only by spaces or a tiny word into one change.
    for (let i = 1; i < segs.length - 1; i++) {
      const s = segs[i];
      if (s.op === "=" && segs[i - 1].op !== "=" && segs[i + 1].op !== "=" && (isBlank(s.text) || visibleLength(s.text) <= 2)) {
        s.op = "~";
      }
    }
    const out = [];
    for (let i = 0; i < segs.length; ) {
      if (segs[i].op === "=") {
        out.push(segs[i++]);
        continue;
      }
      let del = "";
      let ins = "";
      while (i < segs.length && segs[i].op !== "=") {
        const s = segs[i++];
        if (s.op !== "+") del += s.text;
        if (s.op !== "-") ins += s.text;
      }
      if (del) out.push({ op: "-", text: del });
      if (ins) out.push({ op: "+", text: ins });
    }
    return out;
  }

  // Code: lines first, then words inside the changed lines that still resemble each
  // other, so that a word never gets matched to the same word on an unrelated line.
  function lineSegments(oldText, newText) {
    const split = (t) => t.match(/[^\n]*\n|[^\n]+$/g) || [];
    const a = split(oldText);
    const b = split(newText);
    const ops = myers(a, b);
    if (!ops) return [{ op: "-", text: oldText }, { op: "+", text: newText }];
    const out = [];
    const push = (op, text) => {
      const last = out[out.length - 1];
      if (last && last.op === op) last.text += text;
      else out.push({ op, text });
    };
    for (let i = 0; i < ops.length; ) {
      if (ops[i][0] === "=") {
        push("=", a[ops[i++][1]]);
        continue;
      }
      const dels = [];
      const ins = [];
      while (i < ops.length && ops[i][0] !== "=") {
        if (ops[i][0] === "-") dels.push(a[ops[i][1]]);
        else ins.push(b[ops[i][2]]);
        i++;
      }
      let next = 0;
      let pending = [];
      for (const d of dels) {
        let j = next;
        let words = null;
        for (; j < ins.length && j - next < 20; j++) if ((words = wordSegments(d, ins[j]))) break;
        if (!words) {
          pending.push(d);
          continue;
        }
        for (const p of pending) push("-", p);
        pending = [];
        for (; next < j; next++) push("+", ins[next]);
        for (const w of words) push(w.op, w.text);
        next = j + 1;
      }
      for (const p of pending) push("-", p);
      for (; next < ins.length; next++) push("+", ins[next]);
    }
    return out;
  }

  function textNodes(root) {
    const nodes = [];
    const walker = root.ownerDocument.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    let pos = 0;
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      nodes.push({ node: n, start: pos });
      pos += n.length;
    }
    return nodes;
  }

  function span(doc, cls, text) {
    const s = doc.createElement("span");
    s.className = cls;
    if (text !== undefined) s.textContent = text;
    return s;
  }

  // Marks the word changes inside `el` (the new block). False if the block changed
  // too much to be worth showing word by word.
  // mode: "words", "force" (words, however different) or "lines" (code).
  function markWords(oldEl, el, mode = "words") {
    const segs =
      mode === "lines"
        ? lineSegments(oldEl.textContent, el.textContent)
        : wordSegments(oldEl.textContent, el.textContent, mode === "force");
    if (!segs || !segs.some((s) => s.op !== "=")) return false;
    const doc = el.ownerDocument;

    const edits = [];
    let pos = 0;
    for (const s of segs) {
      if (s.op === "=") pos += s.text.length;
      else if (s.op === "-") edits.push({ op: "-", pos, text: s.text });
      else {
        if (!isBlank(s.text)) edits.push({ op: "+", pos, end: pos + s.text.length });
        pos += s.text.length;
      }
    }

    const nodes = textNodes(el);
    // Back to front, so splitting a text node never shifts the offsets still to come.
    // At the same position the insertion goes first, so the deletion lands before it.
    edits.sort((x, y) => y.pos - x.pos || (x.op === "+" ? -1 : 1));
    for (const e of edits) {
      if (e.op === "+") {
        for (let i = nodes.length - 1; i >= 0; i--) {
          const { node, start } = nodes[i];
          const end = start + node.length;
          if (end <= e.pos || start >= e.end) continue;
          const from = Math.max(e.pos, start) - start;
          const to = Math.min(e.end, end) - start;
          if (isBlank(node.data.slice(from, to))) continue;
          if (to < node.length) node.splitText(to);
          const mid = from > 0 ? node.splitText(from) : node;
          const wrap = span(doc, "mdr-ins");
          mid.replaceWith(wrap);
          wrap.appendChild(mid);
          if (from === 0) nodes[i] = { node: doc.createTextNode(""), start };
        }
      } else {
        const del = span(doc, "mdr-del", e.text);
        let target = null;
        for (const rec of nodes) if (rec.start < e.pos) target = rec;
        if (target && target.node.parentNode) {
          const rest = target.node.splitText(Math.min(e.pos - target.start, target.node.length));
          rest.parentNode.insertBefore(del, rest);
        } else {
          // At the very start: the first text node may already sit inside an insertion.
          el.prepend(del);
        }
      }
    }
    return true;
  }

  // --- blocks -----------------------------------------------------------------------

  const CONTAINERS = new Set(["UL", "OL", "BLOCKQUOTE", "TABLE", "THEAD", "TBODY", "DL", "DIV", "LI", "SECTION", "DETAILS"]);

  const elementsOnly = (el) =>
    [...el.childNodes].every((n) => n.nodeType === 1 || (n.nodeType === 3 && isBlank(n.data)));

  function isContainer(a, b) {
    return CONTAINERS.has(a.tagName) && !a.classList.contains("code-block") && elementsOnly(a) && elementsOnly(b);
  }

  // Diagrams and formulas are re-rendered from their text, so they can't hold marks.
  const atomic = (el) => el.matches("pre.mermaid, .math") || !!el.querySelector("pre.mermaid, .math");

  function deletedCopy(doc, el) {
    const copy = doc.importNode(el, true);
    for (const n of [copy, ...copy.querySelectorAll("[id], [name]")]) {
      n.removeAttribute("id");
      n.removeAttribute("name");
    }
    markBlock(copy, "del");
    return copy;
  }

  // A whole added or removed block. The tint goes behind its text only, like the
  // word marks; diagrams, formulas and images (no text to wrap) get it on the element.
  function markBlock(el, kind) {
    el.dataset.mdr = kind;
    const cls = "mdr-" + kind;
    if (atomic(el) || el.matches("img, hr")) {
      el.classList.add(cls);
      return;
    }
    const doc = el.ownerDocument;
    let wrapped = false;
    for (const { node } of textNodes(el)) {
      if (isBlank(node.data)) continue;
      const wrap = span(doc, cls);
      node.replaceWith(wrap);
      wrap.appendChild(node);
      wrapped = true;
    }
    if (!wrapped) el.classList.add(cls);
  }

  // A table row whose cells line up: compared cell by cell, since a removed cell
  // can't be shown inside a row without breaking the table.
  const sameShapeRows = (a, b) => a.tagName === "TR" && b.tagName === "TR" && a.cells.length === b.cells.length;

  function diffCells(doc, d, n) {
    for (let i = 0; i < n.cells.length; i++) {
      const oc = d.cells[i];
      const nc = n.cells[i];
      if (oc.innerHTML === nc.innerHTML) continue;
      if (atomic(oc) || atomic(nc) || !markWords(oc, nc, "force")) {
        markBlock(nc, "ins");
        nc.prepend(span(doc, "mdr-del", oc.textContent), " ");
      }
    }
  }

  // Code is compared line by line in an editor, so even a heavily edited block reads
  // better as word changes than as one block removed and another added.
  const isCode = (el) => el.classList.contains("code-block");

  function similar(a, b) {
    if (a.tagName !== b.tagName) return false;
    if ((isCode(a) && isCode(b)) || isContainer(a, b)) return true;
    if (sameShapeRows(a, b)) return [...a.cells].some((c, i) => c.textContent === b.cells[i].textContent);
    const ta = a.textContent;
    const tb = b.textContent;
    if (!visibleLength(ta) && !visibleLength(tb)) return true;
    if (ta.length + tb.length > 20000) return false;
    return wordSegments(ta, tb) !== null;
  }

  function diffChildren(oldParent, newParent) {
    const doc = newParent.ownerDocument;
    const a = [...oldParent.children];
    const b = [...newParent.children];
    const ops = myers(a.map((e) => e.outerHTML), b.map((e) => e.outerHTML)) || [
      ...a.map((_, i) => ["-", i, -1]),
      ...b.map((_, j) => ["+", -1, j]),
    ];

    for (let i = 0; i < ops.length; ) {
      if (ops[i][0] === "=") {
        i++;
        continue;
      }
      const dels = [];
      const ins = [];
      while (i < ops.length && ops[i][0] !== "=") {
        if (ops[i][0] === "-") dels.push(a[ops[i][1]]);
        else ins.push(b[ops[i][2]]);
        i++;
      }
      const anchor = i < ops.length ? b[ops[i][2]] : null;
      diffRun(doc, newParent, dels, ins, anchor);
    }
  }

  function diffRun(doc, parent, dels, ins, anchor) {
    // Pair each removed block with a similar added one, keeping the order.
    const partner = new Map();
    let next = 0;
    const tryPairs = dels.length * ins.length <= 400;
    for (const d of dels) {
      for (let j = next; j < ins.length; j++) {
        if (tryPairs ? similar(d, ins[j]) : d.tagName === ins[j].tagName) {
          partner.set(d, ins[j]);
          next = j + 1;
          break;
        }
      }
    }
    const paired = new Set(partner.values());

    let pending = [];
    const flush = (before) => {
      for (const d of pending) {
        const copy = deletedCopy(doc, d);
        if (before) parent.insertBefore(copy, before);
        else parent.appendChild(copy);
      }
      pending = [];
    };
    // Removed blocks go before the blocks added in their place, as in a unified diff.
    let firstFree = 0;
    for (const d of dels) {
      const n = partner.get(d);
      if (!n) {
        pending.push(d);
        continue;
      }
      flush(ins[firstFree]);
      firstFree = ins.indexOf(n) + 1;
      if (isContainer(d, n)) diffChildren(d, n);
      else if (sameShapeRows(d, n)) diffCells(doc, d, n);
      else if (atomic(d) || atomic(n) || !markWords(d, n, isCode(d) && isCode(n) ? "lines" : "words")) {
        parent.insertBefore(deletedCopy(doc, d), n);
        markBlock(n, "ins");
      }
    }
    flush(ins[firstFree] || anchor);
    for (const n of ins) if (!paired.has(n)) markBlock(n, "ins");
  }

  // Returns the new document's nodes with the changes against the old one marked.
  function build(oldHtml, newHtml) {
    const parse = (html) => {
      const t = document.createElement("template");
      t.innerHTML = html;
      return t.content;
    };
    const oldRoot = parse(oldHtml);
    const newRoot = parse(newHtml);
    diffChildren(oldRoot, newRoot);
    return newRoot;
  }

  window.mdrDiff = { myers, lineStats, wordSegments, lineSegments, build };
})();
