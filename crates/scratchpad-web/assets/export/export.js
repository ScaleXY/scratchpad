// Viewer for exported/shared emails. Everything it needs is in this file:
// the data block, fonts, icons and the device preview (devices.js).
"use strict";

const DATA = JSON.parse(document.getElementById("scratchpad-data").textContent);
const EMAILS = DATA.emails;

const $ = (sel, root = document) => root.querySelector(sel);
const esc = (value) =>
  String(value ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
/** Only web and mail links become clickable; `javascript:` and friends from emails don't. */
const safeUrl = (url) => (/^(https?:|mailto:)/i.test(String(url ?? "").trim()) ? String(url).trim() : null);
const icon = (name, cls = "") => `<svg class="i ${cls}" aria-hidden="true"><use href="#${name}"/></svg>`;

const store = {
  get(key, fallback) { try { return localStorage.getItem(key) ?? fallback; } catch (_) { return fallback; } },
  set(key, value) { try { localStorage.setItem(key, value); } catch (_) {} },
};

const state = {
  index: 0,
  tab: "preview",
  showMeta: false,
  device: DATA.options.device || store.get("scratchpad-device", "fit"),
  landscape: store.get("scratchpad-landscape", "0") === "1",
};

// ---------------------------------------------------------------- theme

const darkQuery = matchMedia("(prefers-color-scheme: dark)");
function applyTheme() {
  const pref = store.get("scratchpad-theme", "system");
  const dark = pref === "dark" || (pref === "system" && darkQuery.matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}
darkQuery.addEventListener("change", applyTheme);

// ---------------------------------------------------------------- helpers

const fullDate = (value) => new Date(value).toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
const addr = (list) => (list || []).map((a) => (a.name ? `${a.name} <${a.email}>` : a.email)).join(", ");
const spamTone = (score, threshold = 5) => (score >= threshold ? "bad" : score >= threshold / 2 ? "warn" : "ok");

function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

const AVATAR_COLORS = ["#6366f1", "#8b5cf6", "#ec4899", "#f43f5e", "#f97316", "#eab308", "#10b981", "#14b8a6", "#0ea5e9", "#3b82f6"];
function avatar(name, cls = "") {
  const clean = (name || "?").replace(/[^\p{L}\p{N} ]/gu, " ").trim() || "?";
  const parts = clean.split(/\s+/);
  const initials = ((parts[0]?.[0] || "?") + (parts.length > 1 ? parts[parts.length - 1][0] : "")).toUpperCase();
  let hash = 0;
  for (const ch of clean) hash = (hash * 31 + ch.codePointAt(0)) >>> 0;
  return `<div class="avatar ${cls}" style="background:${AVATAR_COLORS[hash % AVATAR_COLORS.length]}">${esc(initials)}</div>`;
}

function emptyState(iconName, title, text) {
  return `<div class="empty"><div class="art">${icon(iconName, "xl")}</div><h3>${esc(title)}</h3>${text ? `<p>${esc(text)}</p>` : ""}</div>`;
}

const sender = (e) => e.from[0] || { name: null, email: e.envelope_from || "unknown" };

// Attachments are embedded as base64 and turned into blob: URLs on demand.
function attachmentUrl(a) {
  if (!a.data) return null;
  if (!a.url) {
    const bytes = Uint8Array.from(atob(a.data), (c) => c.charCodeAt(0));
    a.url = URL.createObjectURL(new Blob([bytes], { type: a.content_type || "application/octet-stream" }));
  }
  return a.url;
}

function previewHtml(e) {
  if (!e.html) {
    return `<pre style="white-space:pre-wrap;font:14px/1.5 ui-monospace,monospace;margin:16px">${esc(e.text || "")}</pre>`;
  }
  let html = e.html;
  for (const a of e.attachments) {
    const url = a.content_id && attachmentUrl(a);
    if (url) html = html.split(`cid:${a.content_id}`).join(url);
  }
  return ScratchpadDevices.withBase(html);
}

// ---------------------------------------------------------------- rendering

const TABS = [
  ["preview", "Preview", () => true],
  ["text", "Text", () => true],
  ["source", "HTML", (e) => e.html],
  ["headers", "Headers", (e) => e.headers],
  ["attachments", "Attachments", (e) => e.attachments.length],
  ["links", "Links", (e) => e.links],
  ["spam", "Spam", (e) => e.spam],
  ["inspector", "Inspector", (e) => e.view_inspector],
  ["raw", "Raw", (e) => e.raw || e.raw_omitted],
];

function render() {
  const app = $("#x-app");
  const multi = EMAILS.length > 1;
  app.className = `x-app${multi ? " multi" : ""}`;
  const e = EMAILS[state.index];
  if (!TABS.some(([key, , show]) => key === state.tab && show(e))) state.tab = "preview";
  const theme = store.get("scratchpad-theme", "system");
  app.innerHTML = `
    <header class="x-top">
      <img class="brand-mark" src="${$('link[rel=icon]').href}" alt="">
      <div class="x-title"><b>${esc(DATA.title)}</b><span>${multi ? `${EMAILS.length} emails · ` : ""}Shared ${esc(fullDate(DATA.exported_at * 1000))}</span></div>
      <div class="segmented icons" role="group" aria-label="Theme">
        ${[["system", "monitor", "System"], ["light", "sun", "Light"], ["dark", "moon", "Dark"]].map(([key, i, label]) =>
          `<button type="button" class="${theme === key ? "on" : ""}" data-theme-pref="${key}" title="${label} theme" aria-pressed="${theme === key}">${icon(i, "sm")}<span>${label}</span></button>`).join("")}
      </div>
    </header>
    ${multi ? `<aside class="list"><div class="emails">${EMAILS.map(row).join("")}</div></aside>` : ""}
    <main class="detail">
      ${multi ? `<div class="x-picker"><select id="x-pick" aria-label="Email">${EMAILS.map((m, i) => `<option value="${i}" ${i === state.index ? "selected" : ""}>${esc(m.subject || "(no subject)")}</option>`).join("")}</select></div>` : ""}
      <div class="detail-head">${head(e)}</div>
      <div class="tab-body" id="tab-body"></div>
      <footer class="x-foot">Exported with <a href="https://github.com/scalexy/scratchpad" target="_blank" rel="noreferrer">Scratchpad</a> ${esc(DATA.generator.replace(/^Scratchpad\s*/, ""))}</footer>
    </main>`;
  renderTab();
}

function row(e, i) {
  const s = sender(e);
  return `<div class="email-row ${i === state.index ? "active" : ""}" data-index="${i}" tabindex="0" aria-current="${i === state.index}">
    <div class="lead">${avatar(s.name || s.email)}</div>
    <div>
      <div class="top"><span class="from">${esc(s.name || s.email)}</span><span class="time">${esc(new Date(e.received_at * 1000).toLocaleDateString())}</span></div>
      <div class="subject">${esc(e.subject || "(no subject)")}</div>
      <div class="preview">${esc((e.text || "").slice(0, 120))}</div>
    </div>
  </div>`;
}

function head(e) {
  const s = sender(e);
  const recipients = [...e.to, ...e.cc].map((a) => a.name || a.email).join(", ");
  const rows = [
    ["From", addr(e.from) || e.envelope_from],
    ["To", addr(e.to)],
    ["Cc", addr(e.cc)],
    ["Bcc", addr(e.bcc)],
    ["Reply-To", addr(e.reply_to)],
    ["Date", e.date ? fullDate(e.date) : ""],
    ["Received", fullDate(e.received_at * 1000)],
    ["Message-ID", e.message_id || ""],
    ["Size", formatSize(e.size)],
  ].filter(([, v]) => v);
  const broken = (e.links || []).filter((l) => l.status.state === "broken").length;
  const badge = (key) => {
    if (key === "attachments") return `<span class="chip">${e.attachments.length}</span>`;
    if (key === "links" && e.links) return broken ? `<span class="chip bad">${broken}</span>` : `<span class="chip">${e.links.length}</span>`;
    if (key === "spam" && e.spam) return `<span class="chip ${spamTone(e.spam.score, e.spam.threshold)}">${e.spam.score.toFixed(1)}</span>`;
    return "";
  };
  return `
    <h1>${esc(e.subject || "(no subject)")}</h1>
    <div class="sender">
      ${avatar(s.name || s.email, "lg")}
      <div class="who">
        <div><span class="name">${esc(s.name || s.email)}</span>${s.name ? `<span class="addr">&lt;${esc(s.email)}&gt;</span>` : ""}</div>
        <div class="to">to ${esc(recipients || "—")}</div>
      </div>
      <div class="when">${esc(e.date ? fullDate(e.date) : fullDate(e.received_at * 1000))}<br>
        <button type="button" class="link-btn" data-action="meta" aria-expanded="${state.showMeta}">${state.showMeta ? "Hide" : "Details"}${icon("chevron-right", "sm")}</button></div>
    </div>
    ${state.showMeta ? `<dl class="meta-grid">${rows.map(([k, v]) => `<dt>${k}</dt><dd>${esc(v)}</dd>`).join("")}</dl>` : ""}
    <div class="tabs" role="tablist">
      ${TABS.filter(([, , show]) => show(e)).map(([key, label]) =>
        `<button type="button" class="tab ${state.tab === key ? "active" : ""}" role="tab" aria-selected="${state.tab === key}" data-tab="${key}">${label}${badge(key)}</button>`).join("")}
    </div>`;
}

let devices = null;

function renderTab() {
  const e = EMAILS[state.index];
  const body = $("#tab-body");
  if (devices) devices.destroy();
  devices = null;
  switch (state.tab) {
    case "preview": {
      const html = previewHtml(e);
      devices = ScratchpadDevices.render(body, {
        device: state.device,
        landscape: state.landscape,
        icon,
        frame: () => `<iframe title="Email preview" sandbox="allow-popups allow-popups-to-escape-sandbox allow-same-origin" referrerpolicy="no-referrer"></iframe>`,
        setup: (frame) => { frame.srcdoc = html; },
        onChange: ({ device, landscape }) => {
          state.device = device;
          state.landscape = landscape;
          store.set("scratchpad-device", device);
          store.set("scratchpad-landscape", landscape ? "1" : "0");
        },
      });
      break;
    }
    case "text":
      body.innerHTML = e.text ? `<pre class="code">${esc(e.text)}</pre>` : emptyState("file-text", "No plain-text part", "");
      break;
    case "source":
      body.innerHTML = `<pre class="code">${esc(e.html)}</pre>`;
      break;
    case "raw":
      body.innerHTML = e.raw
        ? `<pre class="code">${esc(e.raw)}</pre>`
        : emptyState("file-text", "Raw source not included", `The source is ${formatSize(e.raw_omitted)}, too large to embed.`);
      break;
    case "headers":
      body.innerHTML = `
        <table class="grid"><thead><tr><th>Header</th><th>Value</th></tr></thead><tbody>
        ${e.envelope_from !== undefined ? `<tr><td class="name">SMTP MAIL FROM</td><td class="mono">${esc(e.envelope_from || "—")}</td></tr>
        <tr><td class="name">SMTP RCPT TO</td><td class="mono">${esc((e.envelope_to || []).join(", ") || "—")}</td></tr>` : ""}
        ${e.headers.map((h) => `<tr><td class="name">${esc(h.name)}</td><td class="mono">${esc(h.value)}</td></tr>`).join("")}
        </tbody></table>`;
      break;
    case "attachments":
      body.innerHTML = `<div class="pad">${e.attachments.map((a, i) => `
        <div class="card"><div class="card-row">
          <div class="tile">${icon(a.content_type.startsWith("image/") ? "eye" : "paperclip")}</div>
          <div class="grow"><div class="title">${esc(a.filename)} ${a.inline ? `<span class="chip">inline</span>` : ""}</div>
            <div class="sub">${esc(a.content_type)} · ${formatSize(a.size)}</div>
            ${a.omitted ? `<div class="att-omitted">${esc(a.omitted)}</div>` : ""}</div>
          ${a.data ? `<button type="button" class="btn sm" data-open="${i}">${icon("external-link", "sm")}Open</button>
          <button type="button" class="btn sm" data-download="${i}">${icon("download", "sm")}Download</button>` : ""}
        </div></div>`).join("")}</div>`;
      break;
    case "links":
      body.innerHTML = links(e);
      break;
    case "spam":
      body.innerHTML = spam(e.spam);
      break;
    case "inspector": {
      const v = e.view_inspector;
      body.innerHTML = `<div class="pad inspector">
        <h3>View file</h3><div class="box"><pre>${esc(v.view_file || "—")}</pre></div>
        ${v.view_data_html ? `<h3>View data</h3><div class="box"><iframe id="view-data" title="View data" sandbox="allow-scripts"></iframe></div>` : ""}
        ${v.view_source ? `<h3>View source</h3><div class="box"><pre>${esc(v.view_source)}</pre></div>` : ""}
      </div>`;
      const frame = $("#view-data");
      if (frame) frame.srcdoc = `<!doctype html><meta charset=utf-8><body style="margin:0;background:#18171B">${v.view_data_html}</body>`;
      break;
    }
  }
}

function links(e) {
  if (!e.links.length) return emptyState("link", "No links", "This email doesn't contain any links.");
  const status = (s) => {
    switch (s.state) {
      case "pending": return `<span class="chip">Not checked</span>`;
      case "ok": return `<span class="chip ok">${icon("check")}${s.status} ${esc(s.method)}</span>`;
      case "skipped": return `<span class="chip">${esc(s.reason)}</span>`;
      default: return `<span class="chip bad">${s.status ? s.status + " · " : ""}${esc(s.error)}</span>`;
    }
  };
  return `<table class="grid"><thead><tr><th>Status</th><th>URL</th><th>Text</th></tr></thead><tbody>
    ${e.links.map((l) => `<tr><td>${status(l.status)}</td><td class="mono">${safeUrl(l.url) ? `<a href="${esc(safeUrl(l.url))}" target="_blank" rel="noreferrer">${esc(l.url)}</a>` : esc(l.url)}</td><td>${esc(l.text || "")}</td></tr>`).join("")}
    </tbody></table>`;
}

function spam(r) {
  const tone = spamTone(r.score, r.threshold);
  const color = { ok: "var(--ok)", warn: "var(--warn)", bad: "var(--danger)" }[tone];
  const pct = Math.max(3, Math.min(100, (r.score / (r.threshold * 2)) * 100));
  return `<div class="pad">
    <div class="score-card">
      <div class="score" style="color:${color}">${r.score.toFixed(1)}</div>
      <div class="grow">
        <div><span class="chip ${tone}">${r.is_spam ? "Likely spam" : tone === "ok" ? "Looks good" : "Needs attention"}</span>
          <span class="muted small" style="margin-left:6px">threshold ${r.threshold.toFixed(1)} · ${esc(r.engine)}</span></div>
        <div class="meter"><span style="width:${pct}%;background:${color}"></span></div>
      </div>
    </div>
  </div>
  <table class="grid"><thead><tr><th>Points</th><th>Rule</th><th>Description</th></tr></thead><tbody>
  ${r.rules.map((rule) => `<tr><td class="mono">${rule.score.toFixed(1)}</td><td class="mono">${esc(rule.name)}</td><td>${esc(rule.description)}</td></tr>`).join("") || `<tr><td colspan="3" class="muted">No rules matched.</td></tr>`}
  </tbody></table>`;
}

// ---------------------------------------------------------------- events

document.addEventListener("click", (event) => {
  const target = event.target.closest("[data-index],[data-tab],[data-action],[data-theme-pref],[data-open],[data-download]");
  if (!target) return;
  const e = EMAILS[state.index];
  if (target.dataset.index !== undefined) {
    state.index = Number(target.dataset.index);
    render();
    scrollTo(0, 0);
  } else if (target.dataset.tab) {
    state.tab = target.dataset.tab;
    $(".detail-head").innerHTML = head(e);
    renderTab();
  } else if (target.dataset.action === "meta") {
    state.showMeta = !state.showMeta;
    $(".detail-head").innerHTML = head(e);
  } else if (target.dataset.themePref) {
    store.set("scratchpad-theme", target.dataset.themePref);
    applyTheme();
    render();
  } else if (target.dataset.open !== undefined) {
    const url = attachmentUrl(e.attachments[Number(target.dataset.open)]);
    if (url) window.open(url, "_blank", "noopener");
  } else if (target.dataset.download !== undefined) {
    const a = e.attachments[Number(target.dataset.download)];
    const link = document.createElement("a");
    link.href = attachmentUrl(a);
    link.download = a.filename;
    link.click();
  }
});

document.addEventListener("keydown", (event) => {
  if ((event.key === "Enter" || event.key === " ") && event.target.matches(".email-row")) {
    event.preventDefault();
    event.target.click();
  }
});

document.addEventListener("change", (event) => {
  if (event.target.id === "x-pick") {
    state.index = Number(event.target.value);
    render();
  }
});

document.title = EMAILS.length === 1 ? EMAILS[0].subject || DATA.title : DATA.title;
render();
