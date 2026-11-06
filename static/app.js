// Small enhancements for signed-in pages. Everything works without this file.
(function () {
  // The Post button says where the post will go, and springs to its new width.
  var SPRING = "linear(0, 0.009, 0.035 2.1%, 0.141, 0.281 6.7%, 0.723 12.9%, 0.938 16.7%, 1.017, 1.077, 1.121, 1.149 24.3%, 1.159, 1.163, 1.161, 1.154 29.9%, 1.129 32.8%, 1.051 39.6%, 1.017 43.1%, 0.991, 0.977 51%, 0.974 53.8%, 0.975 57.1%, 0.997 69.8%, 1.003 76.9%, 1.004 83.8%, 1)";
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)");
  document.querySelectorAll("form.composer").forEach(function (form) {
    var submit = form.querySelector("[data-submit]");
    if (!submit || submit.dataset.submit === "Save") return;
    var label = submit.querySelector("span");
    var use = submit.querySelector("use");
    function sync(animate) {
      var pub = form.querySelector("input[name=visibility]:checked");
      var isPub = !!pub && pub.value === "public";
      var w0 = submit.getBoundingClientRect().width;
      label.textContent = isPub ? "Post publicly" : "Post privately";
      if (use) use.setAttribute("href", isPub ? "#i-globe" : "#i-lock");
      if (!animate || reduce.matches || !submit.animate) return;
      var w1 = submit.getBoundingClientRect().width;
      submit.getAnimations().forEach(function (a) { a.cancel(); });
      submit.animate([{ width: w0 + "px" }, { width: w1 + "px" }], { duration: 650, easing: SPRING });
      submit.animate([{ scale: "1 1" }, { scale: "1.08 1" }, { scale: "1 1" }], { duration: 520, easing: SPRING });
      label.animate([{ opacity: 0, transform: "translateY(3px)" }, { opacity: 1, transform: "none" }], { duration: 180, easing: "ease-out" });
    }
    form.addEventListener("change", function (e) {
      if (e.target.name === "visibility") sync(true);
    });
    sync(false);
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
