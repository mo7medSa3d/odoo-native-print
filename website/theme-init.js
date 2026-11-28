(function () {
  try {
    var t =
      localStorage.getItem("itbaa_theme") ||
      (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
    document.documentElement.setAttribute("data-theme", t === "dark" ? "dark" : "light");
  } catch (e) {}
})();
