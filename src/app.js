// Live refresh: re-render content in place so scroll position is preserved.
window.mdrRefresh = function () {
  fetch(location.pathname + "?body", { cache: "no-store" })
    .then((r) => r.text())
    .then((html) => {
      document.getElementById("content").innerHTML = html;
    });
};

document.addEventListener("keydown", (e) => {
  if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "o") {
    e.preventDefault();
    window.ipc.postMessage("open");
  }
});
