// Small enhancements for signed-in pages. Everything works without this file.
(function () {
  // The Post button says where the post will go.
  document.querySelectorAll("form.composer").forEach(function (form) {
    var submit = form.querySelector("[data-submit]");
    if (!submit || submit.dataset.submit === "Save") return;
    function sync() {
      var pub = form.querySelector("input[name=visibility]:checked");
      submit.textContent = pub && pub.value === "public" ? "Post publicly" : "Post privately";
    }
    form.addEventListener("change", sync);
    sync();
  });

  // Ctrl/Cmd+Enter submits the plain textarea (CodeMirror handles its own).
  document.addEventListener("keydown", function (e) {
    if (e.key !== "Enter" || !(e.ctrlKey || e.metaKey)) return;
    var t = e.target;
    if (t && t.matches && t.matches("textarea[data-composer]")) {
      e.preventDefault();
      t.form.requestSubmit();
    }
  });

  // Copy public URL, falling back to selecting the printed URL.
  document.querySelectorAll("[data-copy]").forEach(function (b) {
    b.addEventListener("click", function () {
      var label = b.querySelector("span");
      function done(msg) {
        label.textContent = msg;
        b.classList.add("copied");
        setTimeout(function () { label.textContent = "Copy public URL"; b.classList.remove("copied"); }, 1600);
      }
      function fallback() {
        var el = document.querySelector("[data-copy-text]");
        var r = document.createRange();
        r.selectNodeContents(el);
        var s = getSelection();
        s.removeAllRanges();
        s.addRange(r);
        done("Selected, copy it");
      }
      try {
        navigator.clipboard.writeText(b.dataset.copy).then(function () { done("Copied"); }, fallback);
      } catch (e) {
        fallback();
      }
    });
  });
})();
