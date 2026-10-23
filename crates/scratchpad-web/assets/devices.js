// Multi-device (viewport) email previews. Shared by the web app and exported
// .html files, so it must stay dependency-free.
"use strict";

window.ScratchpadDevices = (() => {
  const DEVICES = [
    { id: "fit", label: "Fit", icon: "maximize-2" },
    { id: "desktop", label: "Desktop", icon: "monitor", width: 1280, height: 800 },
    { id: "laptop", label: "Laptop", icon: "laptop", width: 1024, height: 640 },
    { id: "tablet", label: "Tablet", icon: "tablet", width: 768, height: 1024, rotate: true },
    { id: "mobile", label: "Mobile", icon: "smartphone", width: 390, height: 844, rotate: true },
    { id: "all", label: "All", icon: "layout-grid" },
  ];
  // Shown side by side in "All", smallest first.
  const ALL = ["mobile", "tablet", "desktop"];
  const byId = (id) => DEVICES.find((d) => d.id === id) || DEVICES[0];
  const GAP = 28;

  function size(device, landscape) {
    const rotated = landscape && device.rotate;
    return rotated ? [device.height, device.width] : [device.width, device.height];
  }

  /**
   * Renders the device switcher and preview into `root`.
   *  - icon(name, cls): returns icon markup
   *  - frame(): returns the markup of one preview <iframe>
   *  - setup(iframe): called for every iframe once it is in the document
   *  - onChange({device, landscape}): remember the choice
   */
  function render(root, options) {
    let device = byId(options.device).id;
    let landscape = !!options.landscape;
    const icon = options.icon;
    let observer = null;

    const draw = () => {
      if (observer) observer.disconnect();
      const current = byId(device);
      const devices = device === "all" ? ALL.map(byId) : current.width ? [current] : [];
      const hint = current.width
        ? `${size(current, landscape).join(" × ")}`
        : device === "all" ? "Mobile, tablet and desktop" : "Full width";
      root.innerHTML = `
        <div class="dv-bar">
          <div class="segmented dv-switch" role="group" aria-label="Device">
            ${DEVICES.map((d) => `<button type="button" class="${d.id === device ? "on" : ""}" data-device="${d.id}" title="${d.label}${d.width ? ` (${d.width} × ${d.height})` : ""}">${icon(d.icon, "sm")}<span>${d.label}</span></button>`).join("")}
          </div>
          ${current.rotate || device === "all" ? `<button type="button" class="icon-btn dv-rotate ${landscape ? "on" : ""}" title="Rotate (landscape)" aria-pressed="${landscape}">${icon("rotate-ccw")}</button>` : ""}
          <span class="dv-hint">${hint}</span>
        </div>
        <div class="dv-stage ${devices.length ? "framed" : "fit"}">
          ${devices.length
            ? `<div class="dv-row">${devices.map((d) => {
                const [w, h] = size(d, landscape);
                return `<figure class="dv-device dv-${d.id}" data-w="${w}" data-h="${h}">
                  <div class="dv-box"><div class="dv-screen" style="width:${w}px;height:${h}px">${options.frame()}</div></div>
                  <figcaption>${device === "all" ? `<button type="button" class="link-btn" data-device="${d.id}">${icon(d.icon, "sm")}${d.label}</button>` : ""}<span>${w} × ${h}</span><span class="dv-scale"></span></figcaption>
                </figure>`;
              }).join("")}</div>`
            : options.frame()}
        </div>`;

      root.querySelectorAll("[data-device]").forEach((button) =>
        button.addEventListener("click", () => {
          device = button.dataset.device;
          changed();
        }));
      const rotate = root.querySelector(".dv-rotate");
      if (rotate) rotate.addEventListener("click", () => {
        landscape = !landscape;
        changed();
      });

      const stage = root.querySelector(".dv-stage");
      root.querySelectorAll("iframe").forEach((frame) => {
        if (!devices.length) {
          // Fit: grow the frame to the email's height (same-origin sandbox, no scripts).
          frame.addEventListener("load", () => {
            try { frame.style.height = frame.contentDocument.documentElement.scrollHeight + 24 + "px"; } catch (_) {}
          });
        }
        if (options.setup) options.setup(frame);
      });
      if (devices.length) {
        const fit = () => scale(stage);
        observer = new ResizeObserver(fit);
        observer.observe(stage);
        fit();
      }
    };

    const changed = () => {
      if (options.onChange) options.onChange({ device, landscape });
      draw();
    };

    draw();
    return { destroy: () => observer && observer.disconnect() };
  }

  // Scales every device to fit the stage width, keeping their relative sizes.
  function scale(stage) {
    const figures = [...stage.querySelectorAll(".dv-device")];
    if (!figures.length) return;
    const style = getComputedStyle(stage);
    const available = stage.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
    // Gaps between devices don't scale.
    const widths = figures.reduce((sum, f) => sum + Number(f.dataset.w), 0);
    const factor = Math.max(0.1, Math.min(1, (available - GAP * (figures.length - 1)) / widths));
    figures.forEach((figure) => {
      const w = Number(figure.dataset.w), h = Number(figure.dataset.h);
      const box = figure.querySelector(".dv-box");
      // Captions wrap instead of widening the figure past its device.
      figure.style.width = w * factor + "px";
      box.style.width = w * factor + "px";
      box.style.height = h * factor + "px";
      figure.querySelector(".dv-screen").style.transform = `scale(${factor})`;
      figure.querySelector(".dv-scale").textContent = factor < 1 ? `${Math.round(factor * 100)}%` : "";
    });
  }

  /** Adds `<base target="_blank">` so links open outside the preview. */
  function withBase(html) {
    const base = '<base target="_blank">';
    const at = html.search(/<head[^>]*>/i);
    if (at < 0) return base + html;
    const end = html.indexOf(">", at) + 1;
    return html.slice(0, end) + base + html.slice(end);
  }

  return { DEVICES, render, withBase, byId };
})();
