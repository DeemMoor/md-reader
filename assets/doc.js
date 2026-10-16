// Runs inside each document iframe. Talks to the app shell (the parent page) directly.
(() => {
  "use strict";

  const nonce = document.currentScript.nonce;
  const content = document.getElementById("content");
  const root = document.documentElement;
  const MD_EXT = /\.(md|markdown|mdown|mkd|mkdn|mdx)$/i;

  const shell = (() => {
    try {
      return window.parent !== window ? window.parent.mdrShell || null : null;
    } catch {
      return null;
    }
  })();

  if (shell) shell.catchErrors(window, () => "document " + decodeURIComponent(location.pathname));

  const loading = {};
  function loadScript(src) {
    return (loading[src] ||= new Promise((resolve, reject) => {
      const s = document.createElement("script");
      s.nonce = nonce;
      s.src = src;
      s.onload = resolve;
      s.onerror = reject;
      document.head.appendChild(s);
    }));
  }

  function loadCss(href) {
    if (document.querySelector(`link[href="${href}"]`)) return;
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = href;
    document.head.appendChild(link);
  }

  function effectiveTheme() {
    const t = root.dataset.theme;
    if (t === "auto") return matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
    return t;
  }

  // --- enhancements -------------------------------------------------------

  function addAnchors(scope) {
    scope.querySelectorAll("h1[id], h2[id], h3[id], h4[id], h5[id], h6[id]").forEach((h) => {
      if (h.querySelector(":scope > .anchor")) return;
      const a = document.createElement("a");
      a.className = "anchor";
      a.href = "#" + h.id;
      a.textContent = "#";
      a.setAttribute("aria-hidden", "true");
      h.prepend(a);
    });
  }

  function copyText(text) {
    if (navigator.clipboard && window.isSecureContext) {
      return navigator.clipboard.writeText(text).catch(() => legacyCopy(text));
    }
    legacyCopy(text);
    return Promise.resolve();
  }

  function legacyCopy(text) {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
  }

  function addCopyButtons(scope) {
    scope.querySelectorAll(".code-block").forEach((block) => {
      if (block.querySelector(".code-copy") || block.closest(".source-view")) return;
      const btn = document.createElement("button");
      btn.className = "code-copy";
      btn.type = "button";
      btn.textContent = "Копировать";
      btn.addEventListener("click", () => {
        copyText(block.querySelector("pre").innerText).then(() => {
          btn.textContent = "Скопировано";
          setTimeout(() => (btn.textContent = "Копировать"), 1500);
        });
      });
      block.appendChild(btn);
    });
  }

  async function renderMath(scope) {
    const nodes = [...scope.querySelectorAll(".math:not([data-done])")];
    if (!nodes.length) return;
    loadCss("/_app/vendor/katex/katex.min.css");
    try {
      await loadScript("/_app/vendor/katex/katex.min.js");
    } catch {
      return;
    }
    for (const el of nodes) {
      el.dataset.done = "1";
      window.katex.render(el.textContent, el, {
        displayMode: el.classList.contains("math-display"),
        throwOnError: false,
      });
    }
  }

  function mermaidTheme() {
    return { dark: "dark", sepia: "neutral" }[effectiveTheme()] || "default";
  }

  async function renderMermaid(scope, force = false) {
    const nodes = [...scope.querySelectorAll("pre.mermaid")];
    if (!nodes.length) return;
    try {
      await loadScript("/_app/vendor/mermaid/mermaid.min.js");
    } catch {
      return;
    }
    const todo = [];
    for (const el of nodes) {
      if (el.dataset.src === undefined) el.dataset.src = el.textContent;
      else if (!force) continue;
      el.removeAttribute("data-processed");
      el.textContent = el.dataset.src;
      todo.push(el);
    }
    if (!todo.length) return;
    window.mermaid.initialize({ startOnLoad: false, theme: mermaidTheme(), securityLevel: "strict" });
    try {
      await window.mermaid.run({ nodes: todo, suppressErrors: true });
    } catch {
      /* a broken diagram must not break the page */
    }
  }

  function enhance(scope) {
    addAnchors(scope);
    addCopyButtons(scope);
    renderMath(scope);
    renderMermaid(scope);
  }

  // --- live diff ----------------------------------------------------------------
  // The tab remembers the version it first showed (the shell keeps it across
  // navigation); every later version is shown with the changes against it marked.

  const view = new URLSearchParams(location.search).get("view") === "source" ? "source" : "doc";
  const diff = window.mdrDiff;
  let shownRaw = null; // markdown on screen
  let shownHtml = content.innerHTML; // its rendering, without the marks
  let seenMarks = new Map(); // marks already shown, to tell the fresh ones
  let follow = true;
  let seq = 0;
  let localBase = null;

  const base = () => shell?.diffBase(window) || (localBase ||= { raw: null, html: {} });

  async function fetchRaw() {
    const res = await fetch(location.pathname + "?raw", { cache: "no-store" });
    return res.ok ? res.text() : null;
  }

  async function render(raw) {
    const res = await fetch("/_api/render" + (view === "source" ? "?view=source" : ""), {
      method: "POST",
      body: raw,
      cache: "no-store",
    });
    return res.text();
  }

  const isMark = (el) => !!el.dataset.mdr || el.classList.contains("mdr-ins") || el.classList.contains("mdr-del");
  const topMarks = () =>
    [...content.querySelectorAll(".mdr-ins, .mdr-del")].filter((m) => !m.parentElement.closest(".mdr-ins, .mdr-del"));
  const markKey = (m) => (m.classList.contains("mdr-ins") ? "+" : "-") + m.textContent;

  function reportStats() {
    const b = base();
    shell?.diffStats(window, b.raw === null || shownRaw === null ? { add: 0, del: 0 } : diff.lineStats(b.raw, shownRaw));
  }

  // Flags the marks that were not on screen before this update.
  function findFresh() {
    const before = seenMarks;
    seenMarks = new Map();
    const fresh = [];
    for (const m of topMarks()) {
      const key = markKey(m);
      seenMarks.set(key, (seenMarks.get(key) || 0) + 1);
      const left = before.get(key) || 0;
      if (left) before.set(key, left - 1);
      else fresh.push(m);
    }
    return fresh;
  }

  async function sync(initial) {
    const my = ++seq;
    const raw = await fetchRaw();
    if (my !== seq) return;
    if (raw === null) {
      // Gone or unreadable: let the server word the error.
      if (initial) return;
      const res = await fetch(location.pathname + "?body", { cache: "no-store" });
      if (my !== seq) return;
      content.innerHTML = await res.text();
      shownRaw = null;
      enhance(content);
      reportStats();
      shell?.docUpdated(window);
      return;
    }
    const b = base();
    if (initial && (b.raw === null || b.raw === raw)) {
      // The page the server sent is already current.
      b.raw ??= raw;
      b.html[view] ??= shownHtml;
      shownRaw = raw;
      reportStats();
      return;
    }
    if (!initial && raw === shownRaw) return;

    const html = await render(raw);
    if (my !== seq) return;
    let marked = null;
    if (raw !== b.raw) {
      b.html[view] ??= await render(b.raw);
      if (my !== seq) return;
      marked = diff.build(b.html[view], html);
    }
    shownRaw = raw;
    shownHtml = html;
    if (marked) content.replaceChildren(marked);
    else content.innerHTML = html;

    const fresh = initial ? [] : findFresh();
    if (initial) findFresh();
    for (const m of fresh) m.classList.add("mdr-fresh");
    enhance(content);
    reportStats();
    shell?.docUpdated(window);
    if (follow && fresh.length) requestAnimationFrame(() => followTo(fresh));
  }

  // Brings the latest edit into view as a whole, scrolling as little as possible.
  function followTo(marks) {
    const rects = marks.map((m) => (m.closest(BLOCK) || m).getBoundingClientRect());
    const top = Math.min(...rects.map((r) => r.top));
    const bottom = Math.max(...rects.map((r) => r.bottom));
    const margin = 60;
    let delta;
    if (top >= 0 && bottom <= innerHeight) return;
    if (bottom - top > innerHeight - 2 * margin || top < 0) delta = top - margin;
    else delta = bottom - innerHeight + margin;
    // A hidden or covered window draws no frames, so a smooth scroll would stall there.
    const behavior = document.visibilityState === "visible" ? "smooth" : "instant";
    scrollTo({ top: scrollY + delta, behavior });
  }

  // --- going through the changes ---------------------------------------------------

  const BLOCK = "p, li, tr, h1, h2, h3, h4, h5, h6, dt, dd, blockquote, .code-block, table, ul, ol, pre.mermaid";
  let lastHunk = null;
  let lastTop = 0;

  // Changes grouped the way they read: a block, or a run of adjacent changed blocks.
  function hunks() {
    const list = [];
    let prev = null;
    let prevBottom = 0;
    for (const m of topMarks()) {
      // Inside code (and the source view) a block is too coarse: each mark is its own.
      const inCode = !!m.closest("pre") && !m.matches(BLOCK);
      const el = inCode ? m : m.matches(BLOCK) ? m : m.closest(BLOCK) || m;
      const r = el.getBoundingClientRect();
      const joins =
        prev &&
        (prev === el ||
          (prev.nextElementSibling === el && isMark(prev) && isMark(el)) ||
          (inCode && prev.closest("pre") === el.closest("pre") && r.top - prevBottom < 6));
      if (!joins) list.push(el);
      prev = el;
      prevBottom = r.bottom;
    }
    return list;
  }

  function flash(el) {
    el.classList.remove("mdr-flash");
    void el.offsetWidth;
    el.classList.add("mdr-flash");
  }

  function goChange(dir) {
    const list = hunks();
    if (!list.length) return;
    const line = innerHeight / 3;
    const tops = list.map((h) => h.getBoundingClientRect().top);
    let i;
    const cur = list.indexOf(lastHunk);
    if (cur >= 0 && Math.abs(tops[cur] - lastTop) < 2) {
      // Still where the last jump left us (e.g. the page can't scroll further).
      i = cur + dir;
    } else if (dir > 0) {
      i = tops.findIndex((t) => t > line + 2);
    } else {
      i = tops.findLastIndex((t) => t < line - 2);
    }
    if (i < 0 || i >= list.length) i = dir > 0 ? 0 : list.length - 1;
    scrollBy(0, tops[i] - line);
    lastHunk = list[i];
    lastTop = lastHunk.getBoundingClientRect().top;
    flash(lastHunk);
  }

  // Takes what is on screen as the new starting point and drops the marks.
  function actualize() {
    const b = base();
    if (shownRaw === null || b.raw === shownRaw) return;
    // Keep the reading position: deleted blocks vanish, the rest stays in order.
    const kept = [...content.children].filter((c) => c.dataset.mdr !== "del");
    const index = kept.findIndex((c) => c.getBoundingClientRect().bottom > 0);
    const offset = index >= 0 ? kept[index].getBoundingClientRect().top : 0;
    b.raw = shownRaw;
    b.html = { [view]: shownHtml };
    content.innerHTML = shownHtml;
    seenMarks = new Map();
    lastHunk = null;
    const anchor = content.children[index];
    if (anchor) scrollBy(0, anchor.getBoundingClientRect().top - offset);
    enhance(content);
    reportStats();
    shell?.docUpdated(window);
  }

  // --- API for the shell ----------------------------------------------------

  window.mdrDoc = {
    refresh: () => sync(false),
    goChange,
    actualize,
    applySettings(s) {
      const themeChanged = root.dataset.theme !== s.theme;
      root.dataset.theme = s.theme;
      root.dataset.width = s.width;
      root.style.zoom = s.zoom;
      follow = s.follow !== false;
      if (themeChanged) renderMermaid(content, true);
    },
  };

  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    if (root.dataset.theme === "auto") renderMermaid(content, true);
  });

  // --- links ------------------------------------------------------------------

  function onLink(e) {
    const a = e.target.closest("a[href]");
    if (!a || typeof a.href !== "string" || e.defaultPrevented) return;
    const href = a.getAttribute("href");
    if (href.startsWith("#")) return; // in-page anchor: default behavior
    e.preventDefault();
    if (!shell) return;
    const url = new URL(a.href, location.href);
    if (url.origin !== location.origin) {
      shell.post("openExternal", { url: url.href });
      return;
    }
    if (url.pathname === location.pathname) {
      if (url.hash) location.hash = url.hash;
      return;
    }
    const path = decodeURIComponent(url.pathname.slice(1));
    if (MD_EXT.test(path)) {
      shell.openLink(window, path, url.hash, e.ctrlKey || e.metaKey || e.button === 1);
    } else {
      shell.post("openPath", { path });
    }
  }

  document.addEventListener("click", onLink);
  document.addEventListener("auxclick", (e) => e.button === 1 && onLink(e));

  // --- input forwarded to the shell ---------------------------------------------

  if (shell) {
    document.addEventListener("keydown", (e) => shell.handleKey(e));
    document.addEventListener(
      "wheel",
      (e) => {
        if (e.ctrlKey) {
          e.preventDefault();
          shell.zoomBy(e.deltaY < 0 ? 1 : -1);
        }
      },
      { passive: false },
    );
    window.addEventListener("scroll", () => shell.docScrolled(window), { passive: true });
  }

  enhance(content);
  sync(true);
})();
