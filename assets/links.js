// Links between documents, inside a document iframe: previews on hover, the broken-link
// check and the list of documents that link here. Loaded before doc.js, which calls init().
(() => {
  "use strict";

  const MD_EXT = /\.(md|markdown|mdown|mkd|mkdn|mdx)$/i;
  const HEADING = /^H[1-6]$/;
  const sourceView = new URLSearchParams(location.search).get("view") === "source";

  let content = null;
  let renderMath = () => {};
  let panel = null;
  let checkSeq = 0;
  let backlinkSeq = 0;
  let problems = [];
  let backlinks = [];

  const pathOf = (url) => decodeURIComponent(url.pathname.slice(1));
  const docUrl = (path) =>
    "/" + path.replace(/\\/g, "/").split("/").map((p) => encodeURIComponent(p).replace(/%3A/gi, ":")).join("/");
  const baseName = (path) => path.replace(/\\/g, "/").split("/").pop();
  const inDeleted = (el) => !!el.closest(".mdr-del, [data-mdr='del']");

  function el(tag, attrs = {}, ...children) {
    const node = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
      else node.setAttribute(k, v);
    }
    node.append(...children.filter((c) => c !== null && c !== undefined));
    return node;
  }

  // A link inside the app: which document and which part of it.
  function target(a) {
    if (!a.getAttribute("href") || a.classList.contains("anchor")) return null;
    const url = new URL(a.href, location.href);
    if (url.origin !== location.origin) return null;
    return {
      url,
      same: url.pathname === location.pathname,
      md: MD_EXT.test(url.pathname),
      id: url.hash ? decodeURIComponent(url.hash.slice(1)) : "",
    };
  }

  // Rendered bodies of other documents, fetched once per check.
  let bodies = new Map();
  function body(pathname) {
    if (!bodies.has(pathname)) {
      bodies.set(
        pathname,
        fetch(pathname + "?body", { cache: "no-store" })
          .then((r) => (r.ok ? r.text() : null))
          .then((html) => {
            if (html === null) return null;
            const t = document.createElement("template");
            t.innerHTML = html;
            return t.content;
          })
          .catch(() => null),
      );
    }
    return bodies.get(pathname);
  }

  const byId = (root, id) => root.querySelector(`[id="${CSS.escape(id)}"]`);

  // The part of a document a link points to: a heading with its section, another
  // element (a footnote), or the start of the document.
  function section(root, id) {
    const out = document.createDocumentFragment();
    if (!id) {
      for (const child of [...root.children].slice(0, 10)) out.append(child.cloneNode(true));
      return out;
    }
    const start = byId(root, id);
    if (!start) return null;
    out.append(start.cloneNode(true));
    if (HEADING.test(start.tagName)) {
      const level = +start.tagName[1];
      let next = start.nextElementSibling;
      for (let n = 0; next && n < 12; n++, next = next.nextElementSibling) {
        if ((HEADING.test(next.tagName) && +next.tagName[1] <= level) || next.matches(".footnote-definition")) break;
        out.append(next.cloneNode(true));
      }
    }
    return out;
  }

  // --- preview on hover -----------------------------------------------------------

  const preview = el("div", { class: "mdr-preview markdown-body", hidden: "" });
  let showTimer = 0;
  let hideTimer = 0;
  let hoverLink = null;

  function headingText(h) {
    return [...h.childNodes].filter((n) => !(n.classList && n.classList.contains("anchor"))).map((n) => n.textContent).join("").trim();
  }

  function cleanClone(frag, base) {
    frag.querySelectorAll(".anchor, .code-copy").forEach((n) => n.remove());
    frag.querySelectorAll(".mdr-del, [data-mdr='del']").forEach((n) => n.remove());
    frag.querySelectorAll("pre.mermaid").forEach((n) => n.replaceWith(el("p", { class: "mdr-preview-note" }, "Диаграмма Mermaid")));
    frag.querySelectorAll("[id]").forEach((n) => n.removeAttribute("id"));
    frag.querySelectorAll("[src], [href]").forEach((n) => {
      for (const attr of ["src", "href"]) {
        const v = n.getAttribute(attr);
        if (v) n.setAttribute(attr, new URL(v, base).href);
      }
    });
    return frag;
  }

  async function previewFor(a) {
    const t = target(a);
    if (!t) return null;
    if (t.same) {
      if (!t.id) return null;
      const part = section(content, t.id);
      if (!part) return { title: `В документе нет раздела «${t.id}»`, missing: true };
      const first = part.firstElementChild;
      const title = HEADING.test(first.tagName) ? headingText(first) : first.matches(".footnote-definition") ? "Сноска" : "";
      return { title, frag: cleanClone(part, location.href) };
    }
    if (!t.md) return null;
    const root = await body(t.url.pathname);
    const name = baseName(pathOf(t.url));
    if (!root) return { title: `Файл не найден: ${name}`, missing: true };
    const part = section(root, t.id);
    if (!part) return { title: `В файле ${name} нет раздела «${t.id}»`, missing: true };
    const first = part.firstElementChild;
    const title = t.id && first && HEADING.test(first.tagName) ? `${name} › ${headingText(first)}` : name;
    return { title, frag: cleanClone(part, t.url.href) };
  }

  async function show(a) {
    const data = await previewFor(a);
    if (!data || hoverLink !== a) return;
    preview.replaceChildren(el("div", { class: "mdr-preview-title" + (data.missing ? " missing" : "") }, data.title));
    if (data.frag) preview.append(el("div", { class: "mdr-preview-body" }, data.frag));
    preview.hidden = false;
    renderMath(preview);
    place(a);
  }

  function place(a) {
    const r = a.getBoundingClientRect();
    const w = Math.min(520, innerWidth - 24);
    preview.style.width = w + "px";
    preview.style.left = Math.max(12, Math.min(r.left, innerWidth - w - 12)) + "px";
    const h = preview.offsetHeight;
    const below = r.bottom + 8;
    preview.style.top = (below + h <= innerHeight - 8 || r.top < h + 16 ? below : r.top - h - 8) + "px";
  }

  function hide() {
    clearTimeout(showTimer);
    hoverLink = null;
    preview.hidden = true;
  }

  function onOver(e) {
    const a = e.target.closest("a[href]");
    if (!a || a === hoverLink || !(content.contains(a) || panel?.contains(a))) return;
    clearTimeout(hideTimer);
    clearTimeout(showTimer);
    hoverLink = a;
    if (!preview.hidden) preview.hidden = true;
    showTimer = setTimeout(() => show(a), 400);
  }

  function onOut(e) {
    const a = e.target.closest("a[href]");
    if (!a || a !== hoverLink || a.contains(e.relatedTarget)) return;
    clearTimeout(showTimer);
    hideTimer = setTimeout(hide, 250);
  }

  preview.addEventListener("mouseenter", () => clearTimeout(hideTimer));
  preview.addEventListener("mouseleave", () => (hideTimer = setTimeout(hide, 250)));

  // --- broken links -----------------------------------------------------------------

  function markBroken(node, reason) {
    node.classList.add("mdr-broken");
    // Links explain themselves in the hover preview; images need a tooltip.
    if (node.tagName === "IMG") {
      if (!node.dataset.mdrTitle) node.dataset.mdrTitle = node.getAttribute("title") || "";
      node.title = reason;
    }
    problems.push({ node, reason });
  }

  function clearBroken() {
    for (const node of content.querySelectorAll(".mdr-broken")) {
      node.classList.remove("mdr-broken");
      if (!("mdrTitle" in node.dataset)) continue;
      if (node.dataset.mdrTitle) node.title = node.dataset.mdrTitle;
      else node.removeAttribute("title");
      delete node.dataset.mdrTitle;
    }
  }

  async function check() {
    const my = ++checkSeq;
    bodies = new Map();
    clearBroken();
    problems = [];
    const pending = [];
    for (const a of content.querySelectorAll("a[href]")) {
      if (inDeleted(a)) continue;
      const t = target(a);
      if (!t) {
        // Web links get their address as a tooltip: there is no status bar to show it.
        if (/^https?:/.test(a.href) && !a.title) a.title = a.href;
        continue;
      }
      if (t.same) {
        if (t.id && !byId(content, t.id)) markBroken(a, `В документе нет раздела «${t.id}»`);
      } else pending.push({ node: a, t });
    }
    for (const img of content.querySelectorAll("img[src]")) {
      if (inDeleted(img)) continue;
      const url = new URL(img.src, location.href);
      if (url.origin === location.origin) pending.push({ node: img, t: { url, img: true } });
    }
    if (pending.length) {
      const paths = [...new Set(pending.map((p) => pathOf(p.t.url)))];
      let found = [];
      try {
        const res = await fetch("/_api/exists", { method: "POST", body: JSON.stringify(paths), cache: "no-store" });
        found = await res.json();
      } catch {
        return;
      }
      if (my !== checkSeq) return;
      const exists = new Map(paths.map((p, i) => [p, found[i]]));
      for (const { node, t } of pending) {
        const name = baseName(pathOf(t.url));
        if (!exists.get(pathOf(t.url))) {
          markBroken(node, t.img ? `Картинка не найдена: ${name}` : `Файл не найден: ${name}`);
        } else if (t.md && t.id) {
          const root = await body(t.url.pathname);
          if (my !== checkSeq) return;
          if (root && !byId(root, t.id)) markBroken(node, `В файле ${name} нет раздела «${t.id}»`);
        }
      }
    }
    if (my !== checkSeq) return;
    problems.sort((a, b) => (a.node.compareDocumentPosition(b.node) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1));
    renderPanel();
  }

  // --- under the document: problems and backlinks ---------------------------------

  async function loadBacklinks() {
    const my = ++backlinkSeq;
    try {
      const res = await fetch("/_api/backlinks?path=" + encodeURIComponent(pathOf(new URL(location.href))), { cache: "no-store" });
      const list = await res.json();
      if (my !== backlinkSeq) return;
      backlinks = list;
    } catch {
      return;
    }
    renderPanel();
  }

  function highlighted(snippet, text) {
    const i = text ? snippet.indexOf(text) : -1;
    if (i < 0) return [snippet];
    return [snippet.slice(0, i), el("mark", {}, text), snippet.slice(i + text.length)];
  }

  function jumpTo(node) {
    node.scrollIntoView({ block: "center", behavior: "smooth" });
    node.classList.remove("mdr-flash");
    void node.offsetWidth;
    node.classList.add("mdr-flash");
  }

  function renderPanel() {
    if (!panel) {
      panel = el("aside", { class: "mdr-links markdown-body" });
      content.after(panel);
    }
    const parts = [];
    if (problems.length) {
      parts.push(
        el("h2", {}, `Проблемы со ссылками: ${problems.length}`),
        el(
          "ul",
          { class: "mdr-problems" },
          ...problems.map(({ node, reason }) =>
            el(
              "li",
              {},
              el("button", { type: "button", class: "mdr-problem", onclick: () => jumpTo(node) }, (node.textContent || node.getAttribute("alt") || "картинка").trim()),
              " — " + reason,
            ),
          ),
        ),
      );
    }
    if (backlinks.length) {
      parts.push(
        el("h2", {}, `Ссылаются на этот документ: ${backlinks.length}`),
        el(
          "ul",
          { class: "mdr-backlinks" },
          ...backlinks.map((b) =>
            el("li", {}, el("a", { href: docUrl(b.path) }, b.name), el("div", { class: "mdr-snippet" }, ...highlighted(b.snippet, b.text))),
          ),
        ),
      );
    }
    panel.replaceChildren(...parts);
    panel.hidden = parts.length === 0;
  }

  // --- API for doc.js -----------------------------------------------------------------

  window.mdrLinks = {
    init(opts) {
      content = opts.content;
      renderMath = opts.renderMath || renderMath;
      if (sourceView) return;
      document.body.appendChild(preview);
      document.addEventListener("mouseover", onOver);
      document.addEventListener("mouseout", onOut);
      addEventListener("scroll", hide, { passive: true });
      document.addEventListener("mousedown", (e) => !preview.contains(e.target) && hide());
      document.addEventListener("keydown", hide);
      // Other files may have changed while the tab was in the background.
      document.addEventListener("visibilitychange", () => document.visibilityState === "visible" && window.mdrLinks.refresh());
    },
    // The document was (re)rendered.
    refresh() {
      if (!content || sourceView) return;
      hide();
      check();
      loadBacklinks();
    },
  };
})();
