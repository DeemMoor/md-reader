// The app shell: tabs, sidebar (outline / files), keyboard shortcuts.
// Each tab is an iframe showing one rendered document from the mdr:// protocol.
(() => {
  "use strict";

  const INIT = window.MDR_INIT;
  const isWin = INIT.platform === "windows";
  const isMac = INIT.platform === "macos";
  let settings = INIT.settings;

  const $ = (sel) => document.querySelector(sel);
  const tabsEl = $("#tabs");
  const docsEl = $("#docs");
  const welcome = $("#welcome");
  const tocEl = $("#toc");
  const filesEl = $("#files");

  const ICONS = {
    chevron: '<svg viewBox="0 0 16 16"><path d="M6 4l4 4-4 4"/></svg>',
    folder: '<svg viewBox="0 0 16 16"><path d="M1.5 4.5v8h13v-7H7.5L6 4H1.5z"/></svg>',
    file: '<svg viewBox="0 0 16 16"><path d="M3.5 1.5h6l3 3v10h-9z M9.5 1.5v3h3"/></svg>',
    up: '<svg viewBox="0 0 16 16"><path d="M8 13V3M4 7l4-4 4 4"/></svg>',
    prev: '<svg viewBox="0 0 16 16"><path d="M4 10l4-4 4 4"/></svg>',
    next: '<svg viewBox="0 0 16 16"><path d="M4 6l4 4 4-4"/></svg>',
  };

  const post = (cmd, data = {}) => window.ipc.postMessage(JSON.stringify({ cmd, ...data }));

  // Script errors from the shell and the documents go to the native error log.
  let logged = 0;
  function logError(where, error) {
    if (++logged > 20) return;
    const text = error && error.stack ? error.stack : String(error && error.message ? error.message : error);
    post("log", { message: `${where}: ${text}`.slice(0, 2000) });
  }
  function catchErrors(w, where) {
    w.addEventListener("error", (e) => logError(where(), e.error || `${e.message} (${e.filename}:${e.lineno})`));
    w.addEventListener("unhandledrejection", (e) => logError(where(), e.reason));
  }
  catchErrors(window, () => "shell");

  // --- paths ------------------------------------------------------------------

  const slashes = (p) => String(p).replace(/\\/g, "/");
  const norm = (p) => (isWin ? slashes(p).toLowerCase() : slashes(p));
  const nativePath = (p) => (isWin ? p.replace(/\//g, "\\") : p);
  const baseName = (p) => slashes(p).split("/").pop();
  function dirName(p) {
    const s = slashes(p);
    const i = s.lastIndexOf("/");
    if (i <= 0) return "/";
    const d = s.slice(0, i);
    return /^[A-Za-z]:$/.test(d) ? d + "/" : d;
  }
  function docUrl(path, source) {
    const encoded = slashes(path).split("/").map(encodeURIComponent).join("/");
    return location.origin + "/" + encoded + (source ? "?view=source" : "");
  }
  function pathFromUrl(href) {
    const url = new URL(href);
    return url.pathname.length > 1 ? nativePath(decodeURIComponent(url.pathname.slice(1))) : null;
  }

  function debounce(fn, ms) {
    let t;
    return (...args) => {
      clearTimeout(t);
      t = setTimeout(() => fn(...args), ms);
    };
  }

  // --- tabs -------------------------------------------------------------------

  const tabs = [];
  const closed = [];
  let active = null;

  function frameWindow(tab) {
    try {
      return tab.frame.contentWindow;
    } catch {
      return null;
    }
  }

  function currentScroll(tab) {
    try {
      return Math.round(tab.frame.contentWindow.scrollY);
    } catch {
      return 0;
    }
  }

  function createTab(path, { scroll = 0, hash = null } = {}) {
    const frame = document.createElement("iframe");
    frame.className = "doc";
    frame.setAttribute("allow", "clipboard-write");
    const el = document.createElement("div");
    el.className = "tab";
    el.setAttribute("role", "tab");
    el.innerHTML =
      '<span class="tab-title"></span><button class="tab-close" title="Закрыть (Ctrl+W)">×</button>';
    const tab = {
      path, frame, el, source: false, history: [], future: [], pendingScroll: scroll, pendingHash: hash,
      // Live diff: the version each document was first shown in, and its changes since.
      bases: new Map(), stats: { add: 0, del: 0 },
    };

    el.addEventListener("mousedown", (e) => e.button === 1 && e.preventDefault());
    el.addEventListener("auxclick", (e) => e.button === 1 && closeTab(tab));
    el.addEventListener("click", (e) => (e.target.closest(".tab-close") ? closeTab(tab) : activate(tab)));
    frame.addEventListener("load", () => onFrameLoad(tab));

    tabsEl.appendChild(el);
    docsEl.appendChild(frame);
    tabs.push(tab);
    updateTabLabel(tab);
    frame.src = docUrl(path);
    return tab;
  }

  function updateTabLabel(tab) {
    tab.title = baseName(tab.path) + (tab.source ? " — исходник" : "");
    tab.el.querySelector(".tab-title").textContent = tab.title;
    tab.el.title = tab.path;
    if (tab === active) document.title = tab.title;
  }

  function onFrameLoad(tab) {
    const w = frameWindow(tab);
    let path;
    try {
      path = pathFromUrl(w.location.href);
    } catch {
      return;
    }
    if (!path) return;
    tab.path = path;
    tab.source = new URL(w.location.href).searchParams.get("view") === "source";
    updateTabLabel(tab);
    w.mdrDoc?.applySettings(settings);
    if (tab.pendingHash) {
      const id = decodeURIComponent(tab.pendingHash.slice(1));
      w.document.getElementById(id)?.scrollIntoView();
    } else if (tab.pendingScroll) {
      w.scrollTo(0, tab.pendingScroll);
    }
    tab.pendingHash = null;
    tab.pendingScroll = 0;
    if (tab === active) {
      buildToc();
      syncFiles();
      focusActive();
    }
    reportState();
  }

  function activate(tab) {
    active = tab;
    for (const t of tabs) {
      t.el.classList.toggle("active", t === tab);
      t.frame.classList.toggle("active", t === tab);
    }
    welcome.hidden = tabs.length > 0;
    document.title = tab ? tab.title : "MD Reader";
    if (tab) tab.el.scrollIntoView({ block: "nearest", inline: "nearest" });
    buildToc();
    syncFiles();
    updateDiffTools();
    focusActive();
    reportState();
  }

  function focusActive() {
    // Keyboard scrolling needs the document focused, not the shell.
    setTimeout(() => frameWindow(active)?.focus(), 0);
  }

  function open(paths, { background = false } = {}) {
    let last = null;
    for (const path of paths) {
      last = tabs.find((t) => norm(t.path) === norm(path)) || createTab(path);
      post("opened", { path });
    }
    if (last && !background) activate(last);
    else reportState();
  }

  function closeTab(tab) {
    const i = tabs.indexOf(tab);
    if (i < 0) return;
    closed.push({ path: tab.path, scroll: currentScroll(tab) });
    tab.el.remove();
    tab.frame.remove();
    tabs.splice(i, 1);
    if (tab === active) activate(tabs[i] || tabs[i - 1] || null);
    else reportState();
  }

  function reopenClosed() {
    const c = closed.pop();
    if (!c) return;
    const existing = tabs.find((t) => norm(t.path) === norm(c.path));
    activate(existing || createTab(c.path, { scroll: c.scroll }));
  }

  function cycle(delta) {
    if (tabs.length < 2) return;
    const i = tabs.indexOf(active);
    activate(tabs[(i + delta + tabs.length) % tabs.length]);
  }

  function load(tab, path, source) {
    tab.frame.contentWindow.location.replace(docUrl(path, source));
  }

  function navigate(tab, path, hash) {
    tab.history.push({ path: tab.path, scroll: currentScroll(tab), source: tab.source });
    tab.future = [];
    tab.pendingHash = hash || null;
    load(tab, path, false);
    post("opened", { path });
  }

  function goHistory(tab, from, to) {
    const entry = from.pop();
    if (!entry) return;
    to.push({ path: tab.path, scroll: currentScroll(tab), source: tab.source });
    tab.pendingScroll = entry.scroll;
    load(tab, entry.path, entry.source);
  }

  const reportState = debounce(() => {
    post("state", {
      tabs: tabs.map((t) => ({ path: t.path, scroll: currentScroll(t) })),
      active: Math.max(0, tabs.indexOf(active)),
    });
  }, 300);

  tabsEl.addEventListener("wheel", (e) => {
    if (!e.ctrlKey) tabsEl.scrollLeft += e.deltaY;
  });

  // --- live diff ------------------------------------------------------------------

  const diffTools = document.createElement("div");
  diffTools.id = "difftools";
  diffTools.hidden = true;
  diffTools.innerHTML =
    '<span class="diff-count"><span class="add"></span><span class="del"></span></span>' +
    `<button class="icon-btn small" data-cmd="prev_change" title="Предыдущее изменение (Shift+F7)">${ICONS.prev}</button>` +
    `<button class="icon-btn small" data-cmd="next_change" title="Следующее изменение (F7)">${ICONS.next}</button>` +
    '<button class="text-btn follow" data-cmd="follow" title="Прокручивать к месту, где документ только что изменился">Следить</button>' +
    '<button class="text-btn" data-cmd="actualize" title="Принять текущую версию как исходную и снять подсветку (Ctrl+Shift+R)">Актуализировать</button>';
  diffTools.addEventListener("click", (e) => {
    const b = e.target.closest("button[data-cmd]");
    if (b) command(b.dataset.cmd);
    if (b && b.dataset.cmd !== "follow") focusActive();
  });
  docsEl.appendChild(diffTools);

  const hasChanges = (tab) => tab.stats.add > 0 || tab.stats.del > 0;

  function fillCount(box, stats) {
    box.querySelector(".add").textContent = stats.add ? "+" + stats.add : "";
    box.querySelector(".del").textContent = stats.del ? "−" + stats.del : "";
  }

  function updateDiffTools() {
    const on = !!active && hasChanges(active);
    diffTools.hidden = !on;
    if (on) fillCount(diffTools, active.stats);
    diffTools.querySelector(".follow").classList.toggle("on", settings.follow !== false);
  }

  function setDiffStats(tab, stats) {
    tab.stats = stats;
    if (tab === active) updateDiffTools();
  }

  // --- outline ------------------------------------------------------------------

  let tocItems = [];

  function headingText(h) {
    return [...h.childNodes]
      .filter((n) => !(n.classList && n.classList.contains("anchor")))
      .map((n) => n.textContent)
      .join("")
      .trim();
  }

  function buildToc() {
    tocEl.textContent = "";
    tocItems = [];
    const doc = active && frameWindow(active)?.document;
    const headings = doc ? [...doc.querySelectorAll("#content :is(h1, h2, h3, h4, h5, h6)[id]")] : [];
    if (!headings.length) {
      tocEl.innerHTML = `<p class="empty">${active ? "В документе нет заголовков" : "Нет открытых файлов"}</p>`;
      return;
    }
    const minLevel = Math.min(...headings.map((h) => +h.tagName[1]));
    for (const h of headings) {
      const level = +h.tagName[1] - minLevel;
      const a = document.createElement("a");
      a.className = "toc-item lvl-" + level;
      a.href = "#" + h.id;
      a.style.paddingLeft = 12 + level * 14 + "px";
      a.textContent = headingText(h);
      a.title = a.textContent;
      a.addEventListener("click", (e) => {
        e.preventDefault();
        h.scrollIntoView();
        focusActive();
      });
      tocEl.appendChild(a);
      tocItems.push({ h, a });
    }
    updateSpy();
  }

  let spyQueued = false;
  function updateSpy() {
    if (spyQueued) return;
    spyQueued = true;
    requestAnimationFrame(() => {
      spyQueued = false;
      let current = tocItems[0];
      for (const item of tocItems) {
        if (item.h.getBoundingClientRect().top <= 80) current = item;
        else break;
      }
      for (const item of tocItems) item.a.classList.toggle("current", item === current);
      if (current && settings.sidebarPanel === "toc") current.a.scrollIntoView({ block: "nearest" });
    });
  }

  // --- files --------------------------------------------------------------------

  let filesRoot = null;
  let filesFor = null;

  async function listDir(dir) {
    const res = await fetch("/_api/list?dir=" + encodeURIComponent(dir), { cache: "no-store" });
    return res.json();
  }

  function syncFiles() {
    if (!active) {
      filesRoot = filesFor = null;
      filesEl.innerHTML = '<p class="empty">Нет открытых файлов</p>';
      return;
    }
    const dir = dirName(active.path);
    if (dir !== filesFor) {
      filesFor = dir;
      showDir(dir);
    } else {
      markCurrentFile();
    }
  }

  async function showDir(dir) {
    filesRoot = dir;
    const listing = await listDir(dir);
    if (filesRoot !== dir) return;
    filesEl.textContent = "";
    const head = document.createElement("div");
    head.className = "files-head";
    head.innerHTML = `<button class="icon-btn small" title="На уровень выше">${ICONS.up}</button><span class="dir-name"></span>`;
    head.querySelector(".dir-name").textContent = baseName(listing.dir.replace(/[\\/]$/, "")) || listing.dir;
    head.title = listing.dir;
    const up = head.querySelector("button");
    up.disabled = !listing.parent;
    up.addEventListener("click", () => listing.parent && showDir(listing.parent));
    filesEl.appendChild(head);
    const list = document.createElement("div");
    renderEntries(list, listing.entries, 0);
    filesEl.appendChild(list);
    markCurrentFile();
  }

  function renderEntries(container, entries, depth) {
    if (!entries.length && depth === 0) {
      container.innerHTML = '<p class="empty">Здесь нет markdown-файлов</p>';
      return;
    }
    for (const entry of entries) {
      const row = document.createElement("div");
      row.className = "file-row" + (entry.dir ? " is-dir" : "");
      row.style.paddingLeft = 8 + depth * 14 + "px";
      row.dataset.path = norm(entry.path);
      row.title = entry.path;
      row.innerHTML = entry.dir
        ? `<span class="chev">${ICONS.chevron}</span><span class="ico">${ICONS.folder}</span><span class="name"></span>`
        : `<span class="chev"></span><span class="ico">${ICONS.file}</span><span class="name"></span>`;
      row.querySelector(".name").textContent = entry.name;
      container.appendChild(row);
      if (entry.dir) {
        const children = document.createElement("div");
        children.hidden = true;
        container.appendChild(children);
        let loaded = false;
        row.addEventListener("click", async () => {
          row.classList.toggle("open");
          children.hidden = !row.classList.contains("open");
          if (!loaded) {
            loaded = true;
            renderEntries(children, (await listDir(entry.path)).entries, depth + 1);
            markCurrentFile();
          }
        });
      } else {
        row.addEventListener("click", () => open([entry.path]));
        row.addEventListener("auxclick", (e) => e.button === 1 && open([entry.path], { background: true }));
      }
    }
  }

  function markCurrentFile() {
    const current = active ? norm(active.path) : null;
    filesEl.querySelectorAll(".file-row").forEach((r) => r.classList.toggle("current", r.dataset.path === current));
  }

  // --- sidebar ------------------------------------------------------------------

  document.querySelectorAll(".sidebar-tabs button").forEach((b) =>
    b.addEventListener("click", () => setPanel(b.dataset.panel)),
  );

  function setPanel(panel) {
    settings.sidebarPanel = panel;
    applySidebar();
    post("settings", { patch: { sidebarPanel: panel } });
  }

  function applySidebar() {
    document.body.classList.toggle("no-sidebar", !settings.sidebar);
    document.body.style.setProperty("--sidebar-width", settings.sidebarWidth + "px");
    for (const b of document.querySelectorAll(".sidebar-tabs button")) {
      b.classList.toggle("active", b.dataset.panel === settings.sidebarPanel);
    }
    tocEl.classList.toggle("active", settings.sidebarPanel === "toc");
    filesEl.classList.toggle("active", settings.sidebarPanel === "files");
  }

  $("#resizer").addEventListener("mousedown", (e) => {
    e.preventDefault();
    document.body.classList.add("resizing");
    const move = (ev) => {
      settings.sidebarWidth = Math.min(600, Math.max(160, ev.clientX));
      applySidebar();
    };
    const up = () => {
      document.body.classList.remove("resizing");
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", up);
      post("settings", { patch: { sidebarWidth: settings.sidebarWidth } });
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", up);
  });

  // --- settings & zoom ----------------------------------------------------------

  function applySettings(s) {
    settings = s;
    document.documentElement.dataset.theme = s.theme;
    applySidebar();
    for (const t of tabs) frameWindow(t)?.mdrDoc?.applySettings(s);
    renderWelcomeRecent();
    updateDiffTools();
  }

  const ZOOMS = [0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3];
  function zoomBy(dir) {
    let i = ZOOMS.findIndex((z) => z >= settings.zoom - 0.001);
    if (i < 0) i = ZOOMS.length - 1;
    setZoom(ZOOMS[Math.min(ZOOMS.length - 1, Math.max(0, i + dir))]);
  }
  function setZoom(z) {
    settings.zoom = z;
    for (const t of tabs) frameWindow(t)?.mdrDoc?.applySettings(settings);
    saveZoom();
  }
  const saveZoom = debounce(() => post("settings", { patch: { zoom: settings.zoom } }), 400);

  // --- commands & keys ----------------------------------------------------------

  function exportHtml() {
    const doc = active && frameWindow(active)?.document;
    if (!doc) return;
    const clone = doc.getElementById("content").cloneNode(true);
    clone.querySelectorAll(".code-copy, .anchor").forEach((n) => n.remove());
    post("exportHtml", { path: active.path, title: baseName(active.path), html: clone.innerHTML });
  }

  // --- about -------------------------------------------------------------------

  const aboutEl = $("#about");

  function el(tag, attrs = {}, ...children) {
    const node = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (k === "class") node.className = v;
      else if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
      else node.setAttribute(k, v);
    }
    node.append(...children.filter((c) => c !== null && c !== undefined));
    return node;
  }

  function aboutText(a) {
    return [
      `MD Reader ${a.version}`,
      `Сборка: ${a.buildDate}${a.commit ? " (" + a.commit + ")" : ""}`,
      `Система: ${a.os}`,
      `Движок: ${a.engine}`,
      `Программа: ${a.exe}`,
      `Настройки: ${a.settingsFile}`,
      `Данные: ${a.dataDir}`,
    ].join("\n");
  }

  function showAbout() {
    const a = INIT.about;
    const link = (text, onclick, title) => el("a", { href: "#", title: title || text, onclick: (e) => (e.preventDefault(), onclick()) }, text);
    const row = (label, value) => [el("dt", {}, label), el("dd", {}, value)];
    const copyBtn = el("button", { class: "secondary", type: "button" }, "Скопировать сведения");
    copyBtn.addEventListener("click", () => {
      navigator.clipboard.writeText(aboutText(a)).then(() => {
        copyBtn.textContent = "Скопировано";
        setTimeout(() => (copyBtn.textContent = "Скопировать сведения"), 1500);
      });
    });
    const closeBtn = el("button", { class: "primary", type: "button", onclick: hideAbout }, "Закрыть");

    aboutEl.replaceChildren(
      el(
        "div",
        { class: "modal-box", role: "dialog", "aria-modal": "true", "aria-label": "О программе" },
        el(
          "div",
          { class: "about-head" },
          el("img", { src: "/_app/icons/icon-256.png", alt: "" }),
          el(
            "div",
            {},
            el("h2", {}, "MD Reader"),
            el("div", { class: "about-version" }, `версия ${a.version} · ${a.os}`),
          ),
        ),
        el("p", { class: "about-desc" }, "Читалка Markdown-файлов: вкладки, оглавление, темы, подсветка кода, диаграммы Mermaid и формулы."),
        el(
          "dl",
          { class: "about-info" },
          ...row("Автор", "Deem Moor"),
          ...row("Сайт", link("github.com/DeemMoor/md-reader", () => post("openExternal", { url: "https://github.com/DeemMoor/md-reader" }))),
          ...row("Сборка", `${a.buildDate}${a.commit ? " · " + a.commit : ""}`),
          ...row("Движок", a.engine),
        ),
        el("p", { class: "about-credits" }, "© 2026 Deem Moor. Используются pulldown-cmark, syntect, wry, tao, muda, KaTeX и Mermaid."),
        el("div", { class: "modal-actions" }, copyBtn, closeBtn),
      ),
    );
    aboutEl.hidden = false;
    closeBtn.focus();
  }

  function hideAbout() {
    updateBox = null;
    aboutEl.hidden = true;
    focusActive();
  }

  aboutEl.addEventListener("mousedown", (e) => e.target === aboutEl && hideAbout());

  // --- updates -------------------------------------------------------------------

  const updateBtn = $("#update-btn");
  let update = null; // the newer release, once known
  let updateBox = null; // its dialog while open

  function modal(label, ...children) {
    const box = el("div", { class: "modal-box", role: "dialog", "aria-modal": "true", "aria-label": label }, ...children);
    aboutEl.replaceChildren(box);
    aboutEl.hidden = false;
    return box;
  }

  function message(title, text) {
    updateBox = null;
    const ok = el("button", { class: "primary", type: "button", onclick: hideAbout }, "Закрыть");
    modal(title, el("h2", { class: "modal-title" }, title), el("p", {}, text), el("div", { class: "modal-actions" }, ok));
    ok.focus();
  }

  async function renderNotes(target, markdown) {
    if (!markdown.trim()) {
      target.textContent = "Описание изменений не опубликовано.";
      return;
    }
    try {
      const res = await fetch("/_api/render", { method: "POST", body: markdown, cache: "no-store" });
      target.innerHTML = await res.text();
      target.querySelectorAll("a[href]").forEach((a) =>
        a.addEventListener("click", (e) => {
          e.preventDefault();
          post("openExternal", { url: a.href });
        }),
      );
    } catch {
      target.textContent = markdown;
    }
  }

  function showUpdate(r) {
    const notes = el("div", { class: "release-notes" });
    const status = el("p", { class: "update-status" });
    const later = el("button", { class: "secondary", type: "button", onclick: hideAbout }, "Позже");
    const page = el("button", { class: "secondary", type: "button", onclick: () => post("openExternal", { url: r.page }) }, "Страница релиза");
    const install = el("button", { class: "primary", type: "button" }, r.installer ? "Обновить" : "Скачать");
    install.addEventListener("click", () => {
      if (!r.installer) return post("openExternal", { url: r.page });
      install.disabled = later.disabled = true;
      install.textContent = "Загрузка…";
      status.textContent = "";
      post("installUpdate", { url: r.installer, checksums: r.checksums || "" });
    });
    updateBox = { install, later, status };
    modal(
      "Обновление",
      el("h2", { class: "modal-title" }, "Доступна версия " + r.version),
      el("p", { class: "about-version" }, "Сейчас установлена " + INIT.version + ". Что нового:"),
      notes,
      status,
      el("div", { class: "modal-actions" }, page, later, install),
    );
    install.focus();
    renderNotes(notes, r.notes);
  }

  updateBtn.addEventListener("click", () => update && showUpdate(update));

  // Handled by the native side: dialogs, window state, settings that live in the menu.
  const NATIVE = new Set(["open", "new_from_clipboard", "sidebar", "exit", "fullscreen", "shortcuts", "follow"]);

  function command(name) {
    if (NATIVE.has(name)) return post("menu", { id: name });
    const tab = active;
    switch (name) {
      case "close_tab": return tab && closeTab(tab);
      case "reopen_tab": return reopenClosed();
      case "next_tab": return cycle(1);
      case "prev_tab": return cycle(-1);
      case "editor": return tab && post("editor", { path: tab.path });
      case "reveal": return tab && post("reveal", { path: tab.path });
      case "save_as": return tab && post("saveAs", { path: tab.path });
      case "export_html": return exportHtml();
      case "print": return tab && frameWindow(tab)?.print();
      case "show_toc":
      case "show_files":
        setPanel(name === "show_toc" ? "toc" : "files");
        if (!settings.sidebar) post("menu", { id: "sidebar" });
        return;
      case "zoom_in": return zoomBy(1);
      case "zoom_out": return zoomBy(-1);
      case "zoom_reset": return setZoom(1);
      case "source":
        if (!tab) return;
        tab.pendingScroll = 0;
        return load(tab, tab.path, !tab.source);
      case "reload": return tab && frameWindow(tab)?.mdrDoc?.refresh();
      case "next_change": return tab && frameWindow(tab)?.mdrDoc?.goChange(1);
      case "prev_change": return tab && frameWindow(tab)?.mdrDoc?.goChange(-1);
      case "actualize": return tab && frameWindow(tab)?.mdrDoc?.actualize();
      case "about": return showAbout();
      case "show_update": return update && showUpdate(update);
      case "back": return tab && goHistory(tab, tab.history, tab.future);
      case "forward": return tab && goHistory(tab, tab.future, tab.history);
    }
  }

  const MOD_KEYS = {
    KeyO: "open", KeyW: "close_tab", F4: "close_tab", KeyB: "sidebar", KeyE: "editor",
    KeyU: "source", KeyP: "print", KeyR: "reload", KeyQ: "exit",
    Equal: "zoom_in", NumpadAdd: "zoom_in", Minus: "zoom_out", NumpadSubtract: "zoom_out",
    Digit0: "zoom_reset", Numpad0: "zoom_reset", PageDown: "next_tab", PageUp: "prev_tab",
  };

  function keyCommand(e) {
    const mod = isMac ? e.metaKey : e.ctrlKey;
    if (mod && !e.altKey) {
      if (e.code === "Tab") return e.shiftKey ? "prev_tab" : "next_tab";
      if (e.shiftKey) return { KeyT: "reopen_tab", KeyR: "actualize", KeyV: "new_from_clipboard", KeyS: "save_as" }[e.code] || null;
      const digit = /^Digit([1-9])$/.exec(e.code);
      if (digit) {
        const n = +digit[1];
        const tab = n === 9 ? tabs[tabs.length - 1] : tabs[n - 1];
        return tab ? () => activate(tab) : null;
      }
      return MOD_KEYS[e.code] || null;
    }
    if (e.altKey && !mod && !e.shiftKey) {
      if (e.code === "ArrowLeft") return "back";
      if (e.code === "ArrowRight") return "forward";
    }
    if (!mod && !e.altKey && !e.shiftKey) {
      return { F5: "reload", F7: "next_change", F11: "fullscreen", F1: "shortcuts" }[e.code] || null;
    }
    if (e.shiftKey && !mod && !e.altKey) {
      return e.code === "F7" ? "prev_change" : null;
    }
    return null;
  }

  function handleKey(e) {
    if (!aboutEl.hidden) {
      if (e.code === "Escape" && !updateBox?.install.disabled) {
        e.preventDefault();
        hideAbout();
        return true;
      }
      return false;
    }
    const cmd = keyCommand(e);
    if (!cmd) return false;
    e.preventDefault();
    e.stopPropagation();
    if (typeof cmd === "function") cmd();
    else command(cmd);
    return true;
  }

  document.addEventListener("keydown", handleKey);
  document.addEventListener(
    "wheel",
    (e) => {
      if (e.ctrlKey) {
        e.preventDefault();
        zoomBy(e.deltaY < 0 ? 1 : -1);
      }
    },
    { passive: false },
  );

  // --- welcome ------------------------------------------------------------------

  function renderWelcomeRecent() {
    const box = $("#welcome-recent");
    box.textContent = "";
    const recent = (settings.recent || []).slice(0, 8);
    if (!recent.length) return;
    const h = document.createElement("h2");
    h.textContent = "Недавние";
    box.appendChild(h);
    for (const path of recent) {
      const b = document.createElement("button");
      b.className = "recent-item";
      b.innerHTML = '<span class="name"></span><span class="dir"></span>';
      b.querySelector(".name").textContent = baseName(path);
      b.querySelector(".dir").textContent = nativePath(dirName(path));
      b.title = path;
      b.addEventListener("click", () => open([path]));
      box.appendChild(b);
    }
  }

  $("#welcome-open").addEventListener("click", () => command("open"));
  $("#welcome-paste").addEventListener("click", () => command("new_from_clipboard"));
  $("#open-btn").addEventListener("click", () => command("open"));
  $("#sidebar-toggle").addEventListener("click", () => command("sidebar"));

  // --- API for the native side and the documents ----------------------------------

  window.mdrShell = {
    open: (paths) => open(paths),
    command,
    applySettings,
    handleKey,
    zoomBy,
    post,
    catchErrors,
    notify: (title, text) => message(title, text),
    // "Save as" moved a document: point its tabs at the new file, keeping the diff base.
    replacePath(from, to) {
      for (const t of tabs) {
        if (norm(t.path) !== norm(from)) continue;
        const base = t.bases.get(norm(from));
        if (base) t.bases.set(norm(to), base);
        t.path = to;
        load(t, to, t.source);
      }
      post("opened", { path: to });
      reportState();
    },
    fileChanged(path) {
      for (const t of tabs) if (norm(t.path) === norm(path)) frameWindow(t)?.mdrDoc?.refresh();
    },
    diffBase(w) {
      const tab = tabs.find((t) => frameWindow(t) === w);
      const path = tab && pathFromUrl(w.location.href);
      if (!path) return null;
      const key = norm(path);
      if (!tab.bases.has(key)) tab.bases.set(key, { raw: null, html: {} });
      return tab.bases.get(key);
    },
    diffStats(w, stats) {
      const tab = tabs.find((t) => frameWindow(t) === w);
      if (tab) setDiffStats(tab, stats);
    },
    updateResult({ ok, manual, release, error }) {
      if (!ok) {
        if (manual) message("Обновления", `Не удалось проверить обновления: ${error}.`);
        return;
      }
      if (!release.newer) {
        if (manual) message("Обновления", `У вас последняя версия — ${INIT.version}.`);
        return;
      }
      update = release;
      updateBtn.hidden = false;
      updateBtn.title = `Доступна версия ${release.version}`;
      if (manual) showUpdate(release);
    },
    updateProgress(pct) {
      if (updateBox) updateBox.install.textContent = `Загрузка… ${pct}%`;
    },
    updateFailed(error) {
      if (!updateBox) return message("Обновления", `Не получилось обновиться: ${error}.`);
      updateBox.install.disabled = updateBox.later.disabled = false;
      updateBox.install.textContent = "Повторить";
      updateBox.status.textContent = `Не получилось: ${error}.`;
    },
    docUpdated(w) {
      if (active && frameWindow(active) === w) buildToc();
    },
    docScrolled(w) {
      if (active && frameWindow(active) === w) {
        updateSpy();
        reportState();
      }
    },
    openLink(w, path, hash, newTab) {
      const tab = tabs.find((t) => frameWindow(t) === w);
      if (!tab) return;
      path = nativePath(path);
      if (newTab) {
        if (!tabs.some((t) => norm(t.path) === norm(path))) createTab(path, { hash });
        post("opened", { path });
        reportState();
      } else {
        navigate(tab, path, hash);
      }
    },
  };

  // --- start ----------------------------------------------------------------------

  applySettings(settings);
  for (const t of settings.session || []) createTab(t.path, { scroll: t.scroll });
  activate(tabs[Math.min(settings.active || 0, tabs.length - 1)] || null);
  post("ready");
})();
