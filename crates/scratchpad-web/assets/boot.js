// Runs before the first paint (loaded without `defer`): picks the theme so the page
// doesn't flash. Kept out of index.html so the Content Security Policy needs no inline scripts.
"use strict";
(function () {
  var pref = "system";
  try { pref = localStorage.getItem("scratchpad-theme") || "system"; } catch (_) {}
  var dark = pref === "dark" || (pref === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
})();
