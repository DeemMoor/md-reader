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

  // --- API for the shell ----------------------------------------------------

  window.mdrDoc = {
    async refresh() {
      const sep = location.search ? "&" : "?";
      const res = await fetch(location.pathname + location.search + sep + "body", { cache: "no-store" });
      content.innerHTML = await res.text();
      enhance(content);
      shell?.docUpdated(window);
    },
    applySettings(s) {
      const themeChanged = root.dataset.theme !== s.theme;
      root.dataset.theme = s.theme;
      root.dataset.width = s.width;
      root.style.zoom = s.zoom;
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
})();
