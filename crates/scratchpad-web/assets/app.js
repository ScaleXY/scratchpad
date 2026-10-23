// Scratchpad web interface. Plain JS, no build step.
"use strict";

const SHARE = document.querySelector('meta[name="scratchpad-share"]')?.content || "";
const API = SHARE ? `/s/${SHARE}/api` : "/api";

const state = {
  status: null,
  settings: null,
  mailboxes: [],
  remotes: [],
  shares: [],
  mailbox: null, // null = all mailboxes
  emails: [],
  total: 0, // emails matching the list, may be more than are loaded
  search: "",
  checked: new Set(),
  selectedId: null,
  detail: null,
  tab: "html",
  device: localStorage.getItem("scratchpad-device") || "fit",
  landscape: localStorage.getItem("scratchpad-landscape") === "1",
  cloud: null, // { configs, vault }
  cloudShares: [],
  showMeta: false,
  page: null, // "settings" | "shares" | "remotes" | "setup"
  shareTitle: "",
  shareMailbox: null,
};

const $ = (sel, root = document) => root.querySelector(sel);
const esc = (value) =>
  String(value ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const enc = encodeURIComponent;
/** Only web and mail links become clickable; `javascript:` and friends from emails don't. */
const safeUrl = (url) => (/^(https?:|mailto:)/i.test(String(url ?? "").trim()) ? String(url).trim() : "#");
// Emails are loaded in pages so huge mailboxes stay responsive.
const PAGE_SIZE = 500;
const icon = (name, cls = "") => `<svg class="i ${cls}" aria-hidden="true"><use href="/assets/icons.svg#${name}"/></svg>`;

// ---------------------------------------------------------------- theme

const THEME_KEY = "scratchpad-theme";
const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");
const themePreference = () => localStorage.getItem(THEME_KEY) || "system";
function applyTheme() {
  const pref = themePreference();
  const dark = pref === "dark" || (pref === "system" && darkQuery.matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}
darkQuery.addEventListener("change", applyTheme);
applyTheme();

// ---------------------------------------------------------------- helpers

/** Fetches `API + path`; throws the API's `{"error"}` message for error responses. */
async function request(path, options = {}) {
  const init = { ...options, headers: { ...(options.headers || {}) } };
  if (options.json !== undefined) {
    init.method = init.method || "POST";
    init.body = JSON.stringify(options.json);
    init.headers["Content-Type"] = "application/json";
  }
  const response = await fetch(API + path, init);
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try { message = (await response.json()).error || message; } catch (_) {}
    throw new Error(message);
  }
  return response;
}

async function api(path, options = {}) {
  const response = await request(path, options);
  if (response.status === 204 || response.status === 202) return null;
  const type = response.headers.get("content-type") || "";
  return type.includes("json") ? response.json() : response.text();
}

let toastTimer;
function toast(message, error = false) {
  const el = $("#toast");
  el.innerHTML = `${icon(error ? "circle-alert" : "check", "sm")}<span>${esc(message)}</span>`;
  el.className = "toast" + (error ? " error" : "");
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.hidden = true), error ? 6000 : 2500);
}

async function attempt(fn, success) {
  try {
    const result = await fn();
    if (success) toast(success);
    return result;
  } catch (error) {
    toast(error.message, true);
  }
}

function formatDate(ts) {
  const date = new Date(ts * 1000);
  const now = new Date();
  const minutes = Math.round((now - date) / 60000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  if (date.toDateString() === now.toDateString()) return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (now - date < 6 * 86400000) return date.toLocaleDateString([], { weekday: "short" });
  return date.toLocaleDateString([], { month: "short", day: "numeric" });
}

const fullDate = (value) => new Date(value).toLocaleString([], { dateStyle: "medium", timeStyle: "short" });

function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

const addr = (list) => (list || []).map((a) => (a.name ? `${a.name} <${a.email}>` : a.email)).join(", ");

/** Splits "Name <email>" into its parts. */
function parseFrom(value) {
  const match = /^(.*?)\s*<([^>]+)>$/.exec(value || "");
  return match ? { name: match[1].replace(/^"|"$/g, "") || match[2], email: match[2] } : { name: value || "Unknown", email: "" };
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

async function copy(text) {
  try {
    await navigator.clipboard.writeText(text);
    toast("Copied to clipboard");
  } catch (_) {
    prompt("Copy this URL", text);
  }
}

const spamTone = (score, threshold = 5) => (score >= threshold ? "bad" : score >= threshold / 2 ? "warn" : "ok");

function emptyState(iconName, title, text, extra = "") {
  return `<div class="empty"><div class="art">${icon(iconName, "xl")}</div><h3>${title}</h3>${text ? `<p>${text}</p>` : ""}${extra}</div>`;
}

// ---------------------------------------------------------------- loading

async function loadStatus() {
  state.status = await api("/status");
  renderSidebar();
}

async function loadMailboxes() {
  state.mailboxes = await api("/mailboxes");
  renderSidebar();
}

/** Loads the list. `more` appends the next page; otherwise reloads as many as are shown. */
async function loadEmails(more = false) {
  const offset = more ? state.emails.length : 0;
  const limit = more ? PAGE_SIZE : Math.max(PAGE_SIZE, state.emails.length);
  const page = `limit=${limit}&offset=${offset}`;
  let emails;
  if (SHARE) {
    const share = await api(`/share?${page}`);
    state.shareTitle = share.title;
    state.shareMailbox = share.mailbox;
    state.total = share.total;
    emails = share.emails;
  } else {
    const q = state.search ? `&q=${enc(state.search)}` : "";
    const response = await request(state.mailbox === null
      ? `/emails?${page}${q}`
      : `/mailboxes/${enc(state.mailbox)}/emails?${page}${q}`);
    emails = await response.json();
    state.total = Number(response.headers.get("x-total-count")) || emails.length;
  }
  if (more) {
    // Mail that arrived in between shifts the pages; skip what is already shown.
    const shown = new Set(state.emails.map((e) => e.id));
    emails = [...state.emails, ...emails.filter((e) => !shown.has(e.id))];
  }
  state.emails = emails;
  const ids = new Set(state.emails.map((e) => e.id));
  state.checked = new Set([...state.checked].filter((id) => ids.has(id)));
  if (state.selectedId && !ids.has(state.selectedId)) {
    state.selectedId = null;
    state.detail = null;
    renderDetail();
  }
  renderList();
}

async function loadDetail(id) {
  const detail = await api(`/emails/${enc(id)}`);
  if (state.selectedId !== id) return;
  const sameEmail = state.detail && state.detail.summary.id === id;
  state.detail = detail;
  if (!sameEmail && state.tab === "inspector" && !detail.view_inspector) state.tab = "html";
  if (sameEmail && state.tab === "html") renderDetailHead();
  else renderDetail();
}

async function selectEmail(id) {
  state.selectedId = id;
  state.page = null;
  document.getElementById("app").classList.remove("page-mode");
  renderSidebar();
  renderList();
  await attempt(() => loadDetail(id));
  const summary = state.emails.find((e) => e.id === id);
  if (summary && !summary.read && !SHARE) {
    summary.read = true;
    renderList();
    api("/emails/read", { json: { ids: [id], read: true } }).catch(() => {});
  }
}

// ---------------------------------------------------------------- sidebar

function renderSidebar() {
  if (SHARE) return;
  const local = state.mailboxes.filter((m) => m.kind === "local");
  const remote = state.mailboxes.filter((m) => m.kind === "remote");
  const totalUnread = state.mailboxes.reduce((n, m) => n + m.unread, 0);
  const totalEmails = state.mailboxes.reduce((n, m) => n + m.total, 0);
  const trailing = (unread, total) =>
    unread ? `<span class="badge">${unread}</span>` : total ? `<span class="count">${total}</span>` : "";
  const mailbox = (m, iconName) => `
    <button class="side-item ${state.mailbox === m.name && !state.page ? "active" : ""}" data-action="mailbox" data-name="${esc(m.name)}" title="${esc(m.name)}">
      ${icon(iconName)}<span class="name">${esc(m.name)}</span>${trailing(m.unread, m.total)}
    </button>`;
  const nav = (page, iconName, label) =>
    `<button class="side-item ${state.page === page ? "active" : ""}" data-action="page" data-page="${page}">${icon(iconName)}<span class="name">${label}</span></button>`;
  const s = state.status;
  $("#sidebar").innerHTML = `
    <div class="brand"><img src="/assets/icon.svg" alt=""><div><div class="name">Scratchpad</div><div class="tag">Email testing</div></div></div>
    <div class="side-scroll">
      <div class="side-group">
        <button class="side-item ${state.mailbox === null && !state.page ? "active" : ""}" data-action="mailbox" data-name="__all__">
          ${icon("inbox")}<span class="name">All emails</span>${trailing(totalUnread, totalEmails)}
        </button>
      </div>
      <div class="side-group">
        <div class="side-label">Mailboxes</div>
        ${local.map((m) => mailbox(m, "mail")).join("") || `<div class="side-hint">Mailboxes appear automatically when mail arrives.</div>`}
      </div>
      <div class="side-group">
        <div class="side-label">Remote <button class="icon-btn sm" data-action="page" data-page="remotes" title="Add remote mailbox" aria-label="Add remote mailbox">${icon("plus", "sm")}</button></div>
        ${remote.map((m) => mailbox(m, "cloud-download")).join("") || `<div class="side-hint">Pull mail from IMAP or a Scratchpad server.</div>`}
      </div>
      <div class="side-group">
        <div class="side-label">App</div>
        ${nav("setup", "terminal", "Setup")}
        ${nav("shares", "share-2", "Shares")}
        ${nav("cloud", "cloud", "Cloud sharing")}
        ${nav("remotes", "server", "Remote mailboxes")}
        ${nav("settings", "settings", "Settings")}
      </div>
    </div>
    <div class="side-foot">
      ${s ? `<div class="status-chip ${s.smtp_error ? "error" : ""}" title="SMTP server"><span class="dot ${s.smtp_running ? "" : "off"}"></span>
        <span>${esc(s.smtp_error || `smtp://${s.smtp_address}`)}</span></div>` : ""}
      <div class="foot-row">
        <span>v${esc(s?.version || "")}</span>
        ${themeSwitch()}
      </div>
    </div>`;
}

function themeSwitch() {
  const pref = themePreference();
  return `<div class="segmented icons" role="group" aria-label="Theme">
    ${[["system", "monitor"], ["light", "sun"], ["dark", "moon"]].map(([value, iconName]) => {
      const label = `${value[0].toUpperCase() + value.slice(1)} theme`;
      return `<button class="${pref === value ? "on" : ""}" data-action="theme" data-theme="${value}" title="${label}" aria-label="${label}" aria-pressed="${pref === value}">${icon(iconName, "sm")}</button>`;
    }).join("")}
  </div>`;
}

// ---------------------------------------------------------------- list

function renderList() {
  const title = SHARE ? state.shareTitle || "Shared emails" : state.mailbox === null ? "All emails" : state.mailbox;
  const checked = state.checked.size;
  const mailboxActions = !SHARE && state.mailbox !== null
    ? `<button class="icon-btn" data-action="clear-mailbox" title="Empty mailbox">${icon("brush-cleaning")}</button>
       <button class="icon-btn danger" data-action="delete-mailbox" title="Delete mailbox">${icon("trash-2")}</button>`
    : "";
  const allChecked = checked > 0 && checked === state.emails.length;
  $("#list").innerHTML = `
    <div class="list-head">
      <div class="title-row">
        <h2>${esc(title)}<span class="sub">${state.total}</span></h2>
        ${SHARE ? themeSwitch() : ""}
        ${!SHARE && state.emails.length ? `<button class="icon-btn" data-action="share-mailbox" title="${state.mailbox === null ? "Share emails" : "Share inbox"}">${icon("share-2")}</button>` : ""}
        ${mailboxActions}
      </div>
      ${!SHARE ? `<label class="search">${icon("search")}<input id="search" type="search" placeholder="Search emails" aria-label="Search emails" value="${esc(state.search)}" autocomplete="off"><kbd>/</kbd></label>` : ""}
      ${!SHARE && checked ? `<div class="bulk">
        <input type="checkbox" class="cb" data-action="check-all" ${allChecked ? "checked" : ""} aria-label="Select all">
        <span class="grow">${checked} selected</span>
        <button class="btn sm ghost" data-action="mark-read">${icon("mail-open", "sm")}Read</button>
        <button class="btn sm ghost" data-action="share-checked">${icon("share-2", "sm")}Share</button>
        <button class="btn sm ghost" data-action="export-checked" title="Export as a single .html file">${icon("file-down", "sm")}Export</button>
        <button class="btn sm ghost danger" data-action="delete-checked">${icon("trash-2", "sm")}Delete</button>
      </div>` : ""}
    </div>
    <div class="emails">
      ${state.emails.map(renderRow).join("") || (state.search
        ? emptyState("search", "No results", `Nothing matches “${esc(state.search)}”.`)
        : SHARE
          ? emptyState("share-2", "Nothing shared", "")
          : emptyState("inbox", "No emails yet", "Point your app at the SMTP server and new mail shows up here instantly.",
              `<button class="btn" data-action="page" data-page="setup">${icon("terminal", "sm")}Setup &amp; test email</button>`))}
      ${state.emails.length < state.total ? `<div class="more"><button class="btn sm" data-action="load-more">Show more <span class="muted">${state.emails.length} of ${state.total}</span></button></div>` : ""}
    </div>`;
}

function renderRow(email) {
  const chips = [];
  if (state.mailbox === null && !SHARE) chips.push(`<span class="chip">${esc(email.mailbox)}</span>`);
  if (email.has_attachments) chips.push(`<span class="chip">${icon("paperclip")}</span>`);
  if (email.broken_links) chips.push(`<span class="chip bad">${icon("link")}${email.broken_links} broken</span>`);
  if (email.spam_score !== null && email.spam_score !== undefined && email.spam_score >= 2.5)
    chips.push(`<span class="chip ${spamTone(email.spam_score)}">${icon("shield-alert")}${email.spam_score.toFixed(1)}</span>`);
  const from = parseFrom(email.from);
  const checked = state.checked.has(email.id);
  return `
    <div class="email-row ${email.id === state.selectedId ? "active" : ""} ${email.read ? "" : "unread"} ${checked ? "checked" : ""}" data-action="select" data-id="${esc(email.id)}" tabindex="0" aria-current="${email.id === state.selectedId}">
      <div class="lead">
        ${avatar(from.name)}
        ${SHARE ? "" : `<div class="check"><input type="checkbox" class="cb" data-action="check" data-id="${esc(email.id)}" ${checked ? "checked" : ""} aria-label="Select email"></div>`}
      </div>
      <div style="min-width:0">
        <div class="top"><span class="from">${esc(from.name)}</span><span class="time">${formatDate(email.received_at)}</span></div>
        <div class="subject">${esc(email.subject)}</div>
        <div class="preview">${esc(email.preview) || "&nbsp;"}</div>
        ${chips.length ? `<div class="chips">${chips.join("")}</div>` : ""}
      </div>
    </div>`;
}

// ---------------------------------------------------------------- detail

const TABS = [
  ["html", "Preview"],
  ["text", "Text"],
  ["source", "HTML"],
  ["raw", "Raw"],
  ["headers", "Headers"],
  ["attachments", "Attachments"],
  ["links", "Links"],
  ["spam", "Spam"],
  ["inspector", "Inspector"],
];

function renderDetail() {
  const d = state.detail;
  if (!d) {
    $("#detail").innerHTML = state.emails.length
      ? emptyState("mail", "No email selected", "Pick an email from the list. Use <kbd>↑</kbd> <kbd>↓</kbd> to move between emails.")
      : emptyState("mails", "Waiting for mail", "Captured emails will appear here.");
    return;
  }
  $("#detail").innerHTML = `<div class="detail-head" id="detail-head"></div><div class="tab-body" id="tab-body"></div>`;
  renderDetailHead();
  renderTab();
}

function renderDetailHead() {
  const d = state.detail;
  const head = $("#detail-head");
  if (!d || !head) return;
  const s = d.summary;
  const sender = d.from[0] || { name: null, email: d.envelope_from || "unknown" };
  const recipients = [...d.to, ...d.cc].map((a) => a.name || a.email).join(", ") || d.envelope_to.join(", ");
  const rows = [
    ["From", addr(d.from) || d.envelope_from],
    ["To", addr(d.to)],
    ["Cc", addr(d.cc)],
    ["Bcc", addr(d.bcc)],
    ["Reply-To", addr(d.reply_to)],
    ["Date", d.date ? fullDate(d.date) : ""],
    ["Received", fullDate(s.received_at * 1000)],
    ["Mailbox", SHARE ? "" : s.mailbox],
    ["Message-ID", d.message_id || ""],
  ].filter(([, v]) => v);
  const broken = (d.links || []).filter((l) => l.status.state === "broken").length;
  const tabBadge = (key) => {
    if (key === "attachments" && d.attachments.length) return `<span class="chip">${d.attachments.length}</span>`;
    if (key === "links" && d.links) return broken ? `<span class="chip bad">${broken}</span>` : `<span class="chip">${d.links.length}</span>`;
    if (key === "spam" && d.spam) return `<span class="chip ${spamTone(d.spam.score, d.spam.threshold)}">${d.spam.score.toFixed(1)}</span>`;
    return "";
  };
  const link = (href, iconName, title, extra = "") =>
    `<a class="icon-btn" href="${href}" title="${title}" aria-label="${title}" ${extra}>${icon(iconName)}</a>`;
  const base = `${API}/emails/${enc(s.id)}`;
  head.innerHTML = `
    <div class="toolbar">
      ${!SHARE ? `<button class="btn sm ghost" data-action="share-one">${icon("share-2", "sm")}Share</button>
      <button class="btn sm ghost" data-action="forward">${icon("send", "sm")}Forward</button>` : ""}
      <span class="grow"></span>
      <span class="size">${formatSize(s.size)}</span>
      ${link(`${base}/html`, "external-link", "Open HTML in new tab", 'target="_blank" rel="noopener"')}
      ${link(`${base}/raw?download=true`, "download", "Download .eml")}
      ${!SHARE ? `<button class="icon-btn" data-action="export-one" title="Export as a single .html file" aria-label="Export as a single .html file">${icon("file-down")}</button>` : ""}
      ${!SHARE ? `<button class="icon-btn" data-action="recheck" title="Re-run link & spam checks" aria-label="Re-run link and spam checks">${icon("refresh-cw")}</button>
      <span class="sep"></span>
      <button class="icon-btn danger" data-action="delete-one" title="Delete (Backspace)" aria-label="Delete email">${icon("trash-2")}</button>` : ""}
    </div>
    <h1>${esc(s.subject)}</h1>
    <div class="sender">
      ${avatar(sender.name || sender.email, "lg")}
      <div class="who">
        <div><span class="name">${esc(sender.name || sender.email)}</span>${sender.name ? `<span class="addr">&lt;${esc(sender.email)}&gt;</span>` : ""}</div>
        <div class="to">to ${esc(recipients || "—")}${d.bcc.length ? ` · bcc ${esc(d.bcc.map((a) => a.email).join(", "))}` : ""}</div>
      </div>
      <div class="when">${esc(d.date ? fullDate(d.date) : fullDate(s.received_at * 1000))}<br>
        <button class="link-btn" data-action="toggle-meta" aria-expanded="${state.showMeta}">${state.showMeta ? "Hide" : "Details"}${icon("chevron-right", "sm")}</button></div>
    </div>
    ${state.showMeta ? `<dl class="meta-grid">${rows.map(([k, v]) => `<dt>${k}</dt><dd>${esc(v)}</dd>`).join("")}</dl>` : ""}
    <div class="tabs" role="tablist">
      ${TABS.filter(([key]) => key !== "inspector" || d.view_inspector)
        .map(([key, label]) => `<button class="tab ${state.tab === key ? "active" : ""}" role="tab" aria-selected="${state.tab === key}" data-action="tab" data-tab="${key}">${label}${tabBadge(key)}</button>`)
        .join("")}
    </div>`;
}

let devices = null;

function renderTab() {
  const d = state.detail;
  const body = $("#tab-body");
  if (devices) devices.destroy();
  devices = null;
  if (!d || !body) return;
  const id = d.summary.id;
  const base = `${API}/emails/${enc(id)}`;
  switch (state.tab) {
    case "html": {
      if (!d.html) {
        body.innerHTML = emptyState("file-text", "No HTML part", "This email only has plain text.",
          `<button class="btn" data-action="tab" data-tab="text">Show text</button>`);
        break;
      }
      devices = ScratchpadDevices.render(body, {
        device: state.device,
        landscape: state.landscape,
        icon,
        frame: () => `<iframe title="Email preview" sandbox="allow-popups allow-popups-to-escape-sandbox allow-same-origin" referrerpolicy="no-referrer" src="${base}/html"></iframe>`,
        onChange: ({ device, landscape }) => {
          state.device = device;
          state.landscape = landscape;
          localStorage.setItem("scratchpad-device", device);
          localStorage.setItem("scratchpad-landscape", landscape ? "1" : "0");
        },
      });
      break;
    }
    case "text":
      body.innerHTML = d.text ? `<pre class="code">${esc(d.text)}</pre>` : emptyState("file-text", "No plain-text part", "Many clients and spam filters prefer emails that include one.");
      break;
    case "source":
      body.innerHTML = d.html ? `<pre class="code">${esc(d.html)}</pre>` : emptyState("code", "No HTML part", "");
      break;
    case "raw":
      body.innerHTML = `<pre class="code muted">Loading…</pre>`;
      api(`/emails/${enc(id)}/raw`).then(
        (raw) => { if (state.tab === "raw" && state.selectedId === id) body.innerHTML = `<pre class="code">${esc(raw)}</pre>`; },
        (error) => { if (state.tab === "raw" && state.selectedId === id) body.innerHTML = emptyState("circle-alert", "Couldn't load the source", esc(error.message)); });
      break;
    case "headers":
      body.innerHTML = `
        <table class="grid"><thead><tr><th>Header</th><th>Value</th></tr></thead><tbody>
        <tr><td class="name">SMTP MAIL FROM</td><td class="mono">${esc(d.envelope_from || "—")}</td></tr>
        <tr><td class="name">SMTP RCPT TO</td><td class="mono">${esc(d.envelope_to.join(", ") || "—")}</td></tr>
        ${d.headers.map((h) => `<tr><td class="name">${esc(h.name)}</td><td class="mono">${esc(h.value)}</td></tr>`).join("")}
        </tbody></table>`;
      break;
    case "attachments":
      body.innerHTML = d.attachments.length
        ? `<div class="pad">${d.attachments.map((a) => `
            <div class="card"><div class="card-row">
              <div class="tile">${icon(a.content_type.startsWith("image/") ? "eye" : "paperclip")}</div>
              <div class="grow"><div class="title">${esc(a.filename)} ${a.inline ? `<span class="chip">inline${a.content_id ? ` · cid:${esc(a.content_id)}` : ""}</span>` : ""}</div>
                <div class="sub">${esc(a.content_type)} · ${formatSize(a.size)}</div></div>
              <a class="btn sm" href="${base}/attachments/${a.index}" target="_blank" rel="noopener">${icon("external-link", "sm")}Open</a>
              <a class="btn sm" href="${base}/attachments/${a.index}" download="${esc(a.filename)}">${icon("download", "sm")}Download</a>
            </div></div>`).join("")}</div>`
        : emptyState("paperclip", "No attachments", "");
      break;
    case "links":
      body.innerHTML = renderLinks(d);
      break;
    case "spam":
      body.innerHTML = renderSpam(d);
      break;
    case "inspector": {
      const v = d.view_inspector;
      body.innerHTML = v
        ? `<div class="pad inspector">
            <h3>View file</h3><div class="box"><pre>${esc(v.view_file || "—")}</pre></div>
            ${v.view_data_html ? `<h3>View data</h3><div class="box"><iframe title="View data" sandbox="allow-scripts" src="${base}/view-data"></iframe></div>` : ""}
            ${v.view_source ? `<h3>View source</h3><div class="box"><pre>${esc(v.view_source)}</pre></div>` : ""}
          </div>`
        : emptyState("braces", "No debug data", "Install <code>beyondcode/helo-laravel</code> to see the view, data and source behind each mailable.");
      break;
    }
  }
}

function renderLinks(d) {
  if (!d.links) {
    return emptyState("link", "Links not checked",
      state.settings && !state.settings.link_checker ? "The link checker is turned off in Settings." : "The link checker hasn't run for this email.",
      SHARE ? "" : `<button class="btn" data-action="recheck">${icon("refresh-cw", "sm")}Check links now</button>`);
  }
  if (!d.links.length) return emptyState("link", "No links", "This email doesn't contain any links.");
  const status = (s) => {
    switch (s.state) {
      case "pending": return `<span class="chip">Checking…</span>`;
      case "ok": return `<span class="chip ok">${icon("check")}${s.status} ${esc(s.method)}</span>`;
      case "skipped": return `<span class="chip">${esc(s.reason)}</span>`;
      default: return `<span class="chip bad">${s.status ? s.status + " · " : ""}${esc(s.error)}</span>`;
    }
  };
  return `<div class="pad" style="padding-bottom:6px"><p class="muted small" style="margin:0">Each link gets a HEAD request, then a GET request if that fails. Turn the checker off in Settings when you test one-time links.</p></div>
    <table class="grid"><thead><tr><th>Status</th><th>URL</th><th>Text</th></tr></thead><tbody>
    ${d.links.map((l) => `<tr><td>${status(l.status)}</td><td class="mono">${safeUrl(l.url) === "#" ? esc(l.url) : `<a href="${esc(safeUrl(l.url))}" target="_blank" rel="noreferrer">${esc(l.url)}</a>`}</td><td>${esc(l.text || "")}</td></tr>`).join("")}
    </tbody></table>`;
}

function renderSpam(d) {
  const r = d.spam;
  if (!r) {
    return emptyState("shield-alert", "No spam report yet",
      state.settings && !state.settings.spam_check ? "Spam checks are turned off in Settings." : "",
      SHARE ? "" : `<button class="btn" data-action="recheck">${icon("refresh-cw", "sm")}Run spam check</button>`);
  }
  const tone = spamTone(r.score, r.threshold);
  const color = { ok: "var(--ok)", warn: "var(--warn)", bad: "var(--danger)" }[tone];
  const pct = Math.max(3, Math.min(100, (r.score / (r.threshold * 2)) * 100));
  return `<div class="pad">
    ${r.note ? `<div class="note">${icon("circle-alert", "sm")}<span>${esc(r.note)}</span></div>` : ""}
    <div class="score-card">
      <div class="score" style="color:${color}">${r.score.toFixed(1)}</div>
      <div class="grow">
        <div><span class="chip ${tone}">${r.is_spam ? "Likely spam" : tone === "ok" ? "Looks good" : "Needs attention"}</span>
          <span class="muted small" style="margin-left:6px">threshold ${r.threshold.toFixed(1)} · ${esc(r.engine)}</span></div>
        <div class="meter"><span style="width:${pct}%;background:${color}"></span></div>
      </div>
    </div>
    <p class="muted small" style="margin:0">The score is only one signal. Receiving servers also weigh your sending server's reputation, SPF, DKIM and DMARC.</p>
  </div>
  <table class="grid"><thead><tr><th>Points</th><th>Rule</th><th>Description</th></tr></thead><tbody>
  ${r.rules.map((rule) => `<tr><td class="mono">${rule.score.toFixed(1)}</td><td class="mono">${esc(rule.name)}</td><td>${esc(rule.description)}</td></tr>`).join("") || `<tr><td colspan="3" class="muted">No rules matched.</td></tr>`}
  </tbody></table>`;
}

// ---------------------------------------------------------------- pages

async function showPage(page) {
  state.page = page;
  document.getElementById("app").classList.add("page-mode");
  renderSidebar();
  const el = $("#detail");
  try {
    if (page === "settings") {
      state.settings = await api("/settings");
      el.innerHTML = renderSettings();
    } else if (page === "shares") {
      [state.shares, state.cloudShares] = await Promise.all([api("/shares"), api("/cloud/shares")]);
      el.innerHTML = renderShares();
    } else if (page === "cloud") {
      state.cloud = await api("/cloud");
      el.innerHTML = renderCloud();
    } else if (page === "remotes") {
      state.remotes = await api("/remotes");
      el.innerHTML = renderRemotes();
    } else if (page === "setup") {
      el.innerHTML = renderSetup();
    }
  } catch (error) {
    el.innerHTML = emptyState("circle-alert", "Something went wrong", esc(error.message));
  }
}

function leavePage() {
  state.page = null;
  document.getElementById("app").classList.remove("page-mode");
  renderSidebar();
  renderDetail();
}

const pageWrap = (title, lead, body) =>
  `<div class="page"><div class="page-inner"><div class="page-head"><h1>${title}</h1><p>${lead}</p></div>${body}</div></div>`;

/** A settings row. `extra` holds attributes such as `min="1"`; passwords are write-only. */
function field(label, name, value, type = "text", help = "", extra = "") {
  const id = `f-${name.replace(/\W/g, "-")}`;
  const control = type === "checkbox"
    ? `<input type="checkbox" class="switch" id="${id}" name="${name}" ${value ? "checked" : ""}>`
    : type === "password"
      ? `<input type="password" id="${id}" name="${name}" value="" autocomplete="new-password" placeholder="${value ? "Unchanged" : "Not set"}">`
      : `<input type="${type}" id="${id}" name="${name}" value="${esc(value)}" ${type === "number" ? `${extra || 'min="0"'} required` : extra}>`;
  return `<div class="field"><label for="${id}">${label}${help ? `<span class="help">${help}</span>` : ""}</label><div class="ctl">${control}</div></div>`;
}

function select(label, name, value, options, help = "") {
  const id = `f-${name.replace(/\W/g, "-")}`;
  return `<div class="field"><label for="${id}">${label}${help ? `<span class="help">${help}</span>` : ""}</label><div class="ctl"><select id="${id}" name="${name}">${options.map(([v, l]) => `<option value="${v}" ${v === value ? "selected" : ""}>${l}</option>`).join("")}</select></div></div>`;
}

const section = (title, lead, fields) =>
  `<div class="section"><div class="section-head"><h2>${title}</h2>${lead ? `<p>${lead}</p>` : ""}</div>${fields}</div>`;

function renderSettings() {
  const s = state.settings;
  return `<form class="page" id="settings-form"><div class="page-inner">
    <div class="page-head"><h1>Settings</h1><p>Saved to <code>${esc(state.status?.data_dir || "")}</code>.</p></div>
    ${section("SMTP server", "Where your apps deliver mail.",
      field("Host", "smtp_host", s.smtp_host, "text", "0.0.0.0 accepts mail from Docker, VMs and other machines.") +
      field("Port", "smtp_port", s.smtp_port, "number", "", 'min="1" max="65535"') +
      field("Advertise STARTTLS", "smtp_starttls", s.smtp_starttls, "checkbox", "Self-signed certificate. Only for clients that require TLS.") +
      field("Require authentication", "smtp_require_auth", s.smtp_require_auth, "checkbox", "When off, mail without credentials goes to “Default”.") +
      field("Max message size", "max_message_size", s.max_message_size, "number", "In bytes.", 'min="1024"'))}
    ${section("Checks", "Run on every incoming email.",
      field("Link checker", "link_checker", s.link_checker, "checkbox", "Turn off when testing one-time links like email verification.") +
      field("Link timeout (s)", "link_checker_timeout_secs", s.link_checker_timeout_secs, "number", "", 'min="1" max="300"') +
      field("Spam report", "spam_check", s.spam_check, "checkbox", "SpamAssassin's spamd when reachable, otherwise built-in rules.") +
      field("spamd host", "spamd_host", s.spamd_host) +
      field("spamd port", "spamd_port", s.spamd_port, "number", "", 'min="1" max="65535"'))}
    ${section("Forwarding", "SMTP server used to forward captured emails to real inboxes.",
      field("Host", "forward.host", s.forward.host, "text", "e.g. smtp.postmarkapp.com") +
      field("Port", "forward.port", s.forward.port, "number", "", 'min="1" max="65535"') +
      select("Encryption", "forward.encryption", s.forward.encryption, [["none", "None"], ["start_tls", "STARTTLS"], ["tls", "TLS"]]) +
      field("Username", "forward.username", s.forward.username) +
      field("Password", "forward.password", s.forward.password_set, "password") +
      field("From override", "forward.from_override", s.forward.from_override, "text", "Envelope sender. Empty keeps the original."))}
    ${section("Web interface", "Host and port changes apply after a restart.",
      field("Host", "web_host", s.web_host) +
      field("Port", "web_port", s.web_port, "number", "", 'min="1" max="65535"') +
      field("Username", "web_username", s.web_username, "text", "Set a username or password to require basic auth.") +
      field("Password", "web_password", s.web_password_set, "password") +
      field("Public URL", "public_url", s.public_url, "url", "Base URL for share links, e.g. https://scratchpad.example.com"))}
    ${section("General", "",
      field("Desktop notifications", "notifications", s.notifications, "checkbox") +
      select("Desktop app theme", "theme", s.theme, [["system", "System"], ["light", "Light"], ["dark", "Dark"]]) +
      field("Remote poll interval (s)", "remote_poll_secs", s.remote_poll_secs, "number", "", 'min="1"') +
      field("Delete emails after (days)", "retention_days", s.retention_days, "number", "0 keeps emails forever."))}
    <div class="actions-bar"><button class="btn primary" type="submit">${icon("check", "sm")}Save changes</button></div>
  </div></form>`;
}

function readForm(form, base) {
  const out = structuredClone(base);
  for (const el of form.elements) {
    if (!el.name) continue;
    const path = el.name.split(".");
    let target = out;
    for (const key of path.slice(0, -1)) target = target[key];
    const key = path[path.length - 1];
    const current = target[key];
    if (el.type === "checkbox") target[key] = el.checked;
    // Untouched password fields keep the stored password (`null`); a cleared one removes it.
    else if (el.type === "password") target[key] = el.dataset.dirty ? el.value : null;
    else if (typeof current === "number") target[key] = Number(el.value);
    else target[key] = el.value;
  }
  return out;
}

const EXPIRY = [[3600, "1 hour"], [86400, "1 day"], [604800, "7 days"]];

function expiryChip(share) {
  if (share.visibility !== "private" || !share.expires_at) return "";
  const left = share.expires_at - Date.now() / 1000;
  if (left <= 0) return `<span class="chip bad">${icon("clock")}Expired</span>`;
  const text = left < 5400 ? `${Math.ceil(left / 60)}m` : left < 129600 ? `${Math.round(left / 3600)}h` : `${Math.round(left / 86400)}d`;
  return `<span class="chip ${left < 3600 ? "warn" : ""}">${icon("clock")}Expires in ${text}</span>`;
}

function renderShares() {
  const cloud = state.cloudShares.map((c) => `
    <div class="card"><div class="card-row">
      <div class="tile">${icon(c.visibility === "private" ? "lock" : "cloud-upload")}</div>
      <div class="grow">
        <div class="title">${esc(c.title)} <span class="chip ${c.visibility === "private" ? "" : "accent"}">${c.visibility === "private" ? "Private" : "Public"}</span>${expiryChip(c)}</div>
        <div class="sub">${c.emails.length} email${c.emails.length === 1 ? "" : "s"} · ${esc(c.config_name)} · ${formatSize(c.size)} · ${formatDate(c.created_at)}</div>
        <div class="sub mono-sub">${esc(c.visibility === "private" ? c.url.split("?")[0] : c.url)}</div>
      </div>
      <button class="icon-btn" data-action="copy" data-text="${esc(c.url)}" title="Copy link" aria-label="Copy link">${icon("copy")}</button>
      <a class="icon-btn" href="${esc(safeUrl(c.url))}" target="_blank" rel="noreferrer" title="Open" aria-label="Open">${icon("external-link")}</a>
      ${c.visibility === "private" ? `<button class="icon-btn" data-action="renew-cloud-share" data-id="${esc(c.id)}" title="New link" aria-label="New link">${icon("refresh-cw")}</button>` : ""}
      <button class="icon-btn danger" data-action="delete-cloud-share" data-id="${esc(c.id)}" title="Delete" aria-label="Delete share">${icon("trash-2")}</button>
    </div></div>`).join("") || emptyState("cloud-upload", "No cloud shares", "Share an email to an S3 bucket to get a link that works anywhere, even when Scratchpad is offline.",
      `<button class="btn" data-action="page" data-page="cloud">${icon("cloud", "sm")}Set up cloud sharing</button>`);
  const server = state.shares.map(({ share, url }) => `
    <div class="card"><div class="card-row">
      <div class="tile">${icon("share-2")}</div>
      <div class="grow"><div class="title">${esc(share.title)}</div>
        <div class="sub">${share.mailbox ? `Entire inbox “${esc(share.mailbox)}”, live` : `${share.email_ids.length} email${share.email_ids.length === 1 ? "" : "s"}`} · ${formatDate(share.created_at)} · ${esc(url || "")}</div></div>
      ${url ? `<button class="icon-btn" data-action="copy" data-text="${esc(url)}" title="Copy link" aria-label="Copy link">${icon("copy")}</button>
        <a class="icon-btn" href="${esc(safeUrl(url))}" target="_blank" rel="noopener" title="Open" aria-label="Open">${icon("external-link")}</a>` : ""}
      <button class="icon-btn danger" data-action="delete-share" data-token="${esc(share.token)}" title="Revoke" aria-label="Revoke share">${icon("trash-2")}</button>
    </div></div>`).join("") || `<p class="muted small">No server links. Choose “Live link from this server” when sharing.</p>`;
  return pageWrap("Shares", "Every share is read-only. Delete a share to revoke it.",
    `<h2 class="group first">Cloud shares</h2>
     <p class="muted small group-lead">Uploaded to S3 as a single .html file with the viewer built in. Private links expire, at most after 7 days.</p>
     ${cloud}
     <h2 class="group">Server links</h2>
     <p class="muted small group-lead">Served by this Scratchpad while it runs. Inbox links also show emails that arrive later.</p>
     ${server}`);
}

const MODES = { public: "Public links", private: "Expiring private links", both: "Public or expiring links" };

function renderCloud() {
  const { configs, vault } = state.cloud;
  const cards = configs.map((c) => `
    <div class="card"><div class="card-row">
      <div class="tile">${icon(c.storage === "synced" ? "cloud" : "hard-drive")}</div>
      <div class="grow">
        <div class="title">${esc(c.name)} <span class="chip">${esc(MODES[c.mode] || c.mode)}</span><span class="chip ${c.storage === "synced" ? "accent" : ""}">${c.storage === "synced" ? "Synced" : "This device"}</span></div>
        <div class="sub">${esc(c.endpoint)} / ${esc(c.bucket)}${c.cdn_domain ? ` · ${esc(c.cdn_domain)}` : ""}</div>
      </div>
      <button class="btn sm" data-action="test-s3" data-id="${esc(c.id)}">Test</button>
      <button class="btn sm" data-action="edit-s3" data-id="${esc(c.id)}">Edit</button>
      <button class="icon-btn danger" data-action="delete-s3" data-id="${esc(c.id)}" title="Delete" aria-label="Delete S3 config">${icon("trash-2")}</button>
    </div></div>`).join("") || emptyState("cloud", "No S3 configs", "Add an S3-compatible bucket (AWS S3, Cloudflare R2, MinIO, Backblaze B2…) to share emails as hosted .html files.");
  const locked = vault.exists && !vault.unlocked
    ? `<div class="note">${icon("lock", "sm")}<span>Synced configs are hidden until you unlock the vault.</span></div>` : "";

  const state_ = !vault.exists ? ["No master password", "Synced configs are encrypted with a master password before they leave this device."]
    : vault.unlocked ? ["Unlocked", `${vault.synced_configs} synced config${vault.synced_configs === 1 ? "" : "s"}${vault.updated_at ? ` · changed ${formatDate(vault.updated_at)}` : ""}`]
    : ["Locked", "Enter the master password to use synced configs."];
  const sync = vault.sync.last_error
    ? `<div class="err">Last sync failed: ${esc(vault.sync.last_error)}</div>`
    : vault.sync.last_sync_at ? `<div class="sub">Last synced ${formatDate(vault.sync.last_sync_at)}</div>` : "";
  const actions = !vault.exists
    ? `<button class="btn primary" data-action="vault-create">${icon("key-round", "sm")}Set master password</button>
       <button class="btn" data-action="vault-restore">${icon("cloud-download", "sm")}Restore from cloud</button>`
    : vault.unlocked
      ? `<button class="btn" data-action="vault-lock">${icon("lock", "sm")}Lock</button>
         <button class="btn" data-action="vault-sync">${icon("refresh-cw", "sm")}Sync now</button>
         <button class="btn" data-action="vault-password">${icon("key-round", "sm")}Change password</button>
         <button class="btn danger" data-action="vault-reset">Reset</button>`
      : `<button class="btn primary" data-action="vault-unlock">${icon("lock-open", "sm")}Unlock</button>
         <button class="btn" data-action="vault-restore">${icon("cloud-download", "sm")}Restore from cloud</button>
         <button class="btn danger" data-action="vault-reset">Reset</button>`;
  return pageWrap("Cloud sharing",
    "Shared emails are exported as a single .html file and uploaded to your bucket: <code>&lt;host&gt;/email/shares/&lt;uuid&gt;.html</code>. Private shares go to <code>email/private/</code> and open through presigned, expiring links.",
    `${locked}${cards}
    <div class="actions-bar" style="justify-content:flex-start"><button class="btn primary" data-action="edit-s3" data-id="">${icon("plus", "sm")}Add S3 config</button></div>
    <h2 class="group">Credential vault</h2>
    <div class="card"><div class="card-row">
      <div class="tile">${icon(vault.unlocked ? "lock-open" : "lock")}</div>
      <div class="grow"><div class="title">${state_[0]}</div><div class="sub">${state_[1]}</div>${sync}</div>
    </div>
    <div class="card-actions">${actions}</div></div>
    <div class="note subtle">${icon("cloud", "sm")}<span><b>${esc(vault.backend.name)}.</b> ${esc(vault.backend.note)} Configs stored on “This device” never leave it and work offline.</span></div>`);
}

function renderRemotes() {
  const cards = state.remotes.map((r) => `
    <div class="card"><div class="card-row">
      <div class="tile">${icon(r.source.type === "imap" ? "at-sign" : "server")}</div>
      <div class="grow">
        <div class="title">${esc(r.name)} <span class="chip">${r.source.type === "imap" ? "IMAP" : "Scratchpad"}</span>${r.enabled ? "" : `<span class="chip warn">Paused</span>`}</div>
        <div class="sub">${r.source.type === "imap" ? `${esc(r.source.username)} @ ${esc(r.source.host)}:${esc(r.source.port)}/${esc(r.source.folder)}` : `${esc(r.source.base_url)} → ${esc(r.source.mailbox)}`}
          · ${r.last_poll_at ? `checked ${formatDate(r.last_poll_at)}` : "not checked yet"}</div>
        ${r.last_error ? `<div class="err">${esc(r.last_error)}</div>` : ""}
      </div>
      <button class="btn sm" data-action="edit-remote" data-id="${esc(r.id)}">Edit</button>
      <button class="icon-btn danger" data-action="delete-remote" data-id="${esc(r.id)}" title="Delete" aria-label="Delete remote mailbox">${icon("trash-2")}</button>
    </div></div>`).join("") || emptyState("cloud-download", "No remote mailboxes", "Test live and staging apps from here.");
  return pageWrap("Remote mailboxes",
    "Scratchpad polls each remote mailbox and shows new mail like a local one. Pull from an IMAP account, or from another Scratchpad running headless on your server.",
    `${cards}<div class="actions-bar" style="justify-content:flex-start">
      <button class="btn primary" data-action="edit-remote" data-id="">${icon("plus", "sm")}Add remote mailbox</button>
      <button class="btn" data-action="poll-remotes">${icon("refresh-cw", "sm")}Check now</button></div>`);
}

function renderSetup() {
  const s = state.status || {};
  const [host, port] = (s.smtp_address || "127.0.0.1:2525").split(/:(?=\d+$)/);
  const shownHost = host === "0.0.0.0" ? "127.0.0.1" : host;
  const snippets = [
    ["Laravel", ".env", `MAIL_MAILER=smtp\nMAIL_HOST=${shownHost}\nMAIL_PORT=${port}\nMAIL_USERNAME=Inbox-Name\nMAIL_PASSWORD=null\nMAIL_ENCRYPTION=null`],
    ["Symfony", ".env", `MAILER_DSN=smtp://Mailbox-Name:@${shownHost}:${port}`],
    ["Nodemailer", "JavaScript", `nodemailer.createTransport({\n  host: "${shownHost}",\n  port: ${port},\n  auth: { user: "Mailbox-Name", pass: "" },\n});`],
    ["Django", "settings.py", `EMAIL_HOST = "${shownHost}"\nEMAIL_PORT = ${port}\nEMAIL_HOST_USER = "Mailbox-Name"\nEMAIL_HOST_PASSWORD = ""`],
    ["Ruby on Rails", "config/environments", `config.action_mailer.delivery_method = :smtp\nconfig.action_mailer.smtp_settings = {\n  user_name: 'Mailbox-Name', password: '',\n  address: '${shownHost}', port: ${port},\n  authentication: :plain\n}`],
    ["WordPress", "functions.php", `add_action('phpmailer_init', function ($m) {\n  $m->isSMTP();\n  $m->Host = '${shownHost}';\n  $m->Port = ${port};\n  $m->SMTPAuth = true;\n  $m->Username = 'Mailbox-Name';\n  $m->Password = '';\n});`],
    ["Go", "net/smtp", `auth := smtp.PlainAuth("", "Mailbox-Name", "", "${shownHost}")\nsmtp.SendMail("${shownHost}:${port}", auth, from, to, msg)`],
    ["Docker & VMs", "hosts", `Docker:  host.docker.internal\nVagrant: 10.0.2.2\n\nSet the SMTP host to 0.0.0.0 in Settings\nso Scratchpad accepts these connections.`],
  ];
  return pageWrap("Setup",
    "Point any app that sends mail over SMTP at Scratchpad. The SMTP username picks the mailbox and any password works. Mail sent without credentials lands in “Default”.",
    `<div class="hero">
      <div class="tile">${icon("mail")}</div>
      <div class="grow"><div class="lbl">SMTP server</div><div class="big">${esc(shownHost)}:${esc(port)}</div></div>
      <input id="test-mailbox" class="input" value="Test" aria-label="Mailbox" title="Mailbox for the test email">
      <button class="btn primary" data-action="test-email">${icon("send", "sm")}Send test email</button>
    </div>
    <div class="snippets">
      ${snippets.map(([title, file, code]) => `<div class="snippet"><div class="snippet-head"><span>${esc(title)} <span class="muted" style="font-weight:400">· ${esc(file)}</span></span>
        <button class="icon-btn sm" data-action="copy" data-text="${esc(code)}" title="Copy" aria-label="Copy ${esc(title)} snippet">${icon("copy", "sm")}</button></div><pre>${esc(code)}</pre></div>`).join("")}
    </div>
    <h2 class="group">Laravel view inspector</h2>
    <p class="muted" style="margin:0">Run <code>composer require --dev beyondcode/helo-laravel</code> to see the view file, view data and Blade source of every mailable in the Inspector tab.</p>`);
}

// ---------------------------------------------------------------- dialogs

/**
 * Shows the dialog. The primary button submits (also with Enter) and runs `onSubmit`, which
 * returns `false` to keep the dialog open. Cancel buttons are `type="button"` so Enter never
 * picks them; other `type="button"` buttons with a `value` (e.g. "test") call `onSubmit`
 * without validating the form.
 */
function openDialog(html, onSubmit) {
  const dialog = $("#dialog");
  const form = $("#dialog-form");
  form.innerHTML = html;
  const run = async (button) => {
    const ok = await onSubmit(form, button);
    if (ok !== false && button?.value !== "test") dialog.close();
  };
  form.onsubmit = (event) => {
    event.preventDefault();
    run(event.submitter || form.querySelector("button[type=submit]"));
  };
  form.querySelectorAll("button[type=button][value]").forEach((button) =>
    button.addEventListener("click", () => (button.value === "cancel" ? dialog.close() : run(button))));
  // Label every control of the two-column layouts.
  form.querySelectorAll(".grid2 > label").forEach((label, i) => {
    const next = label.nextElementSibling;
    const control = next?.matches("input, select") ? next : next?.querySelector("input, select");
    if (!control || label.htmlFor) return;
    control.id ||= `dlg-${i}`;
    label.htmlFor = control.id;
  });
  dialog.showModal();
  const first = form.querySelector("input:not([type=checkbox]), textarea, select");
  if (first) first.focus();
}

const cancelButton = `<button type="button" class="btn" value="cancel">Cancel</button>`;
const dialogButtons = (label) =>
  `<div class="row">${cancelButton}<button type="submit" class="btn primary" value="ok">${label}</button></div>`;

// Shares to an S3 config (exported .html file) or as a link served by this Scratchpad.
// `mailbox` offers a live link to the whole inbox.
async function shareDialog(ids, defaultTitle, mailbox = null) {
  const cloud = await attempt(() => api("/cloud"));
  const configs = cloud?.configs || [];
  const emails = state.emails.filter((e) => ids === null || ids.includes(e.id));
  const pick = ids === null || ids.length > 1;
  const describe = (c) => ({ public: "Public link", private: "Expiring private link", both: "Public or expiring link" })[c.mode];
  const destinations = [
    ...configs.map((c, i) => `<label class="choice"><input type="radio" name="dest" value="${esc(c.id)}" data-kind="cloud" data-mode="${esc(c.mode)}" ${i === 0 ? "checked" : ""}>
      <span><b>${icon(c.storage === "synced" ? "cloud" : "cloud-upload", "sm")} ${esc(c.name)}</b><small>${describe(c)} · uploads a single .html file to ${esc(c.bucket)}</small></span></label>`),
    `<label class="choice"><input type="radio" name="dest" value="server" data-kind="server" ${configs.length ? "" : "checked"}>
      <span><b>${icon("share-2", "sm")} Live link from this server</b><small>Works while Scratchpad runs and is reachable${state.status?.web_url ? ` (${esc(state.status.web_url)})` : ""}.</small></span></label>`,
  ];
  const hint = !configs.length
    ? `<p class="muted small">Add an S3 bucket in <a href="#" data-action="page" data-page="cloud">Cloud sharing</a> to share links that work anywhere.</p>`
    : cloud.vault.exists && !cloud.vault.unlocked ? `<p class="muted small">Unlock the vault in <a href="#" data-action="page" data-page="cloud">Cloud sharing</a> to use synced configs.</p>` : "";
  openDialog(`
    <h3>${mailbox ? "Share inbox" : "Share"}</h3>
    <p class="lead">Creates a read-only link. Anyone with the link can view the shared emails.</p>
    <label class="lbl" for="share-title">Title</label>
    <input id="share-title" name="title" value="${esc(defaultTitle)}">
    <label class="lbl">Destination</label>
    <div class="choices compact">${destinations.join("")}</div>
    ${hint}
    <div class="cloud-only">
      <div class="grid2">
        <span class="vis-row lbl2">Link</span>
        <div class="vis-row segmented" role="radiogroup">
          <label><input type="radio" name="visibility" value="public" checked>Public</label>
          <label><input type="radio" name="visibility" value="private">Expiring</label>
        </div>
        <label class="exp-row" for="share-expires">Expires after</label>
        <select class="exp-row" id="share-expires" name="expires">${EXPIRY.map(([v, l]) => `<option value="${v}" ${v === 86400 ? "selected" : ""}>${l}</option>`).join("")}</select>
        <label for="share-attachments">Attachments</label><div><input type="checkbox" class="switch" id="share-attachments" name="attachments" checked></div>
        <label for="share-technical">Technical details</label><div class="inline"><input type="checkbox" class="switch" id="share-technical" name="technical" checked><span class="muted small">Headers, raw source, Bcc, spam &amp; link reports</span></div>
      </div>
    </div>
    ${mailbox ? `<div class="server-only choices">
      <label class="choice"><input type="radio" name="scope" value="inbox" checked>
        <span><b>Entire inbox</b><small>Includes emails that arrive after the link is created.</small></span></label>
      <label class="choice"><input type="radio" name="scope" value="emails">
        <span><b>Only these emails</b><small>Pick the emails below. Later emails stay private.</small></span></label>
    </div>` : ""}
    ${pick ? `<div class="pick">${emails.map((e) => `<label><input type="checkbox" class="cb" name="id" value="${esc(e.id)}" checked><span>${esc(e.subject)}</span></label>`).join("")}</div>` : ""}
    ${dialogButtons(`${icon("link", "sm")}<span class="submit-label">Create link</span>`)}`,
    async (form, button) => {
      const title = form.title.value || defaultTitle;
      const dest = form.querySelector("input[name=dest]:checked");
      const chosen = pick ? [...form.querySelectorAll("input[name=id]:checked")].map((i) => i.value) : ids;
      const whole = dest.dataset.kind === "server" && mailbox && form.querySelector("input[name=scope]:checked").value === "inbox";
      if (!whole && !chosen.length) {
        toast("Select at least one email", true);
        return false;
      }
      if (dest.dataset.kind === "server") {
        const result = await attempt(() => api("/shares", { json: whole ? { title, mailbox } : { title, ids: chosen } }));
        if (!result) return false;
        if (result.url) await copy(result.url);
        else toast("Share created. Set a public URL in Settings to get a link.");
        return;
      }
      const visibility = form.querySelector("input[name=visibility]:checked").value;
      button.disabled = true;
      button.querySelector(".submit-label").textContent = "Uploading…";
      const result = await attempt(() => api("/cloud/shares", { json: {
        title, ids: chosen, config_id: dest.value, visibility,
        expires_in_secs: visibility === "private" ? Number(form.expires.value) : null,
        options: { attachments: form.attachments.checked, technical: form.technical.checked },
      } }));
      button.disabled = false;
      button.querySelector(".submit-label").textContent = "Upload & copy link";
      if (!result) return false;
      await copy(result.url);
      if (state.page === "shares") showPage("shares");
    });
  // Show only the options that apply to the chosen destination.
  const form = $("#dialog-form");
  const sync = () => {
    const dest = form.querySelector("input[name=dest]:checked");
    const cloud = dest.dataset.kind === "cloud";
    form.querySelector(".cloud-only").hidden = !cloud;
    const serverOnly = form.querySelector(".server-only");
    if (serverOnly) serverOnly.hidden = cloud;
    const mode = dest.dataset.mode;
    if (cloud) {
      const vis = form.querySelector(`input[name=visibility][value=${mode === "private" ? "private" : mode === "public" ? "public" : form.querySelector("input[name=visibility]:checked").value}]`);
      vis.checked = true;
      form.querySelectorAll(".vis-row").forEach((el) => (el.hidden = mode !== "both"));
      form.querySelectorAll(".exp-row").forEach((el) => (el.hidden = vis.value !== "private"));
    }
    const wholeInbox = !cloud && mailbox && form.querySelector("input[name=scope]:checked").value === "inbox";
    const list = form.querySelector(".pick");
    if (list) list.hidden = wholeInbox;
    form.querySelector(".submit-label").textContent = cloud ? "Upload & copy link" : "Create link";
  };
  form.addEventListener("change", sync);
  sync();
}

function s3Dialog(config) {
  const c = config || { id: "", name: "", endpoint: "", region: "auto", bucket: "", access_key: "", cdn_domain: "", mode: "public", path_style: true, storage: "local", has_secret: false };
  openDialog(`
    <h3>${c.id ? "Edit" : "Add"} S3 config</h3>
    <p class="lead">Any S3-compatible storage: AWS S3, Cloudflare R2, MinIO, Backblaze B2, Wasabi…</p>
    <div class="grid2">
      <label>Name</label><input name="name" value="${esc(c.name)}" placeholder="Team bucket" required>
      <label>S3 endpoint</label><input name="endpoint" value="${esc(c.endpoint)}" placeholder="https://s3.eu-west-1.amazonaws.com" required>
      <label>Region</label><input name="region" value="${esc(c.region)}" placeholder="auto">
      <label>Bucket</label><input name="bucket" value="${esc(c.bucket)}" required>
      <label>Access key</label><input name="access_key" value="${esc(c.access_key)}" autocomplete="off" required>
      <label>Secret key</label><input name="secret_key" type="password" autocomplete="new-password" placeholder="${c.has_secret ? "Unchanged" : ""}" ${c.has_secret ? "" : "required"}>
      <label>Vanity/CDN domain</label><input name="cdn_domain" value="${esc(c.cdn_domain)}" placeholder="Optional, e.g. share.example.com">
      <label>Links</label><select name="mode">${Object.entries(MODES).map(([v, l]) => `<option value="${v}" ${c.mode === v ? "selected" : ""}>${l}</option>`).join("")}</select>
      <label>Path-style URLs</label><div class="inline"><input type="checkbox" class="switch" name="path_style" ${c.path_style ? "checked" : ""}><span class="muted small">Off for AWS virtual-hosted buckets</span></div>
      <label>Credentials</label><select name="storage">
        <option value="local" ${c.storage === "local" ? "selected" : ""}>This device only</option>
        <option value="synced" ${c.storage === "synced" ? "selected" : ""}>Synced, encrypted with master password</option></select>
    </div>
    <p class="muted small">Public links need <code>email/shares/</code> to be publicly readable (bucket policy, public bucket or CDN). Keep <code>email/private/</code> private.</p>
    <div id="s3-checks"></div>
    <div class="row"><button type="button" class="btn" value="test">${icon("refresh-cw", "sm")}Test connection</button><span class="grow"></span>
      ${cancelButton}<button type="submit" class="btn primary" value="ok">Save</button></div>`,
    async (form, button) => {
      const body = {
        id: c.id, name: form.name.value, endpoint: form.endpoint.value, region: form.region.value || "auto",
        bucket: form.bucket.value, access_key: form.access_key.value, secret_key: form.secret_key.value,
        cdn_domain: form.cdn_domain.value, mode: form.mode.value, path_style: form.path_style.checked, storage: form.storage.value,
      };
      if (button && button.value === "test") {
        if (!form.endpoint.value || !form.bucket.value || !form.access_key.value) {
          toast("Enter the endpoint, bucket and access key first", true);
          return false;
        }
        button.disabled = true;
        $("#s3-checks").innerHTML = `<div class="checks"><div class="muted">Testing…</div></div>`;
        const checks = await attempt(() => api("/cloud/configs/test", { json: body }));
        button.disabled = false;
        $("#s3-checks").innerHTML = checks ? renderChecks(checks) : "";
        return false;
      }
      const saved = await attempt(() => api("/cloud/configs", { json: body }), "S3 config saved");
      if (!saved) return false;
      showPage("cloud");
    });
}

function renderChecks(checks) {
  return `<div class="checks">${checks.map((c) => `<div class="${c.ok ? "ok" : "bad"}">${icon(c.ok ? "circle-check" : "circle-x")}
    <span><b>${esc(c.name)}</b>${c.detail ? `<small>${esc(c.detail)}</small>` : ""}</span></div>`).join("")}</div>`;
}

function passwordDialog(title, lead, fields, label, submit) {
  openDialog(`
    <h3>${title}</h3>
    <p class="lead">${lead}</p>
    ${fields.map(([name, text]) => `<label class="lbl" for="pw-${name}">${text}</label><input id="pw-${name}" name="${name}" type="password" autocomplete="off" required>`).join("")}
    ${dialogButtons(`<span class="submit-label">${label}</span>`)}`,
    async (form, button) => {
      const values = Object.fromEntries(fields.map(([name]) => [name, form[name].value]));
      if (values.confirm !== undefined && values.confirm !== (values.password ?? values.new)) {
        toast("The passwords don't match", true);
        return false;
      }
      button.disabled = true;
      const ok = await submit(values);
      button.disabled = false;
      if (ok === undefined) return false;
      showPage("cloud");
    });
}

function forwardDialog() {
  openDialog(`
    <h3>Forward email</h3>
    <p class="lead">Sends the original message through the SMTP server configured in Settings → Forwarding.</p>
    <label class="lbl" for="forward-to">Recipients</label>
    <input id="forward-to" name="to" placeholder="client@example.com, colleague@example.com" required>
    ${dialogButtons(`${icon("send", "sm")}Forward`)}`,
    async (form) => {
      const to = form.to.value.split(",").map((s) => s.trim()).filter(Boolean);
      const result = await attempt(() => api(`/emails/${enc(state.selectedId)}/forward`, { json: { to } }), "Email forwarded");
      return result !== undefined;
    });
}

function remoteDialog(remote) {
  const r = remote || { id: "", name: "", enabled: true, source: { type: "imap", host: "", port: 993, tls: true, username: "", password: "", folder: "INBOX", delete_after_fetch: false } };
  const src = r.source;
  openDialog(`
    <h3>${r.id ? "Edit" : "Add"} remote mailbox</h3>
    <p class="lead">New mail is pulled every few seconds and shown like a local mailbox.</p>
    <div class="grid2">
      <label>Name</label><input name="name" value="${esc(r.name)}" placeholder="Staging" required>
      <label>Source</label><select name="type" id="remote-type">
        <option value="imap" ${src.type === "imap" ? "selected" : ""}>IMAP account</option>
        <option value="scratchpad" ${src.type === "scratchpad" ? "selected" : ""}>Scratchpad server</option></select>
      <label>Enabled</label><div><input type="checkbox" class="switch" name="enabled" ${r.enabled ? "checked" : ""}></div>
    </div>
    <div class="grid2 imap-only">
      <label>Host</label><input name="host" value="${esc(src.host || "")}" placeholder="imap.example.com">
      <label>Port</label><input name="port" type="number" value="${esc(src.port || 993)}">
      <label>TLS</label><div><input type="checkbox" class="switch" name="tls" ${src.tls !== false ? "checked" : ""}></div>
      <label>Username</label><input name="username" value="${esc(src.username || "")}">
      <label>Password</label><input name="password" type="password" autocomplete="new-password" placeholder="${src.type === "imap" && src.has_password ? "Unchanged" : ""}">
      <label>Folder</label><input name="folder" value="${esc(src.folder || "INBOX")}">
      <label>Delete after fetch</label><div><input type="checkbox" class="switch" name="delete_after_fetch" ${src.delete_after_fetch ? "checked" : ""}></div>
    </div>
    <div class="grid2 scratchpad-only">
      <label>Server URL</label><input name="base_url" placeholder="https://scratchpad.staging.example.com" value="${esc(src.base_url || "")}">
      <label>Mailbox</label><input name="mailbox" value="${esc(src.mailbox || "Default")}">
      <label>Username</label><input name="h_username" value="${esc(src.type === "scratchpad" ? src.username || "" : "")}" placeholder="optional">
      <label>Password</label><input name="h_password" type="password" autocomplete="new-password" placeholder="${src.type === "scratchpad" && src.has_password ? "Unchanged" : "optional"}">
    </div>
    ${dialogButtons("Save")}`,
    async (form) => {
      const type = form.type.value;
      // Passwords are write-only: `null` keeps the stored one unless the field was edited.
      const secret = (input) => (input.dataset.dirty ? input.value : null);
      const source = type === "imap"
        ? { type, host: form.host.value, port: Number(form.port.value), tls: form.tls.checked, username: form.username.value, password: secret(form.password), folder: form.folder.value || "INBOX", delete_after_fetch: form.delete_after_fetch.checked }
        : { type, base_url: form.base_url.value, mailbox: form.mailbox.value, username: form.h_username.value || null, password: secret(form.h_password) };
      if (type === "imap" ? !source.host : !/^https?:\/\//i.test(source.base_url)) {
        toast(type === "imap" ? "Enter the IMAP host" : "The server URL must start with http:// or https://", true);
        return false;
      }
      const body = { id: r.id, name: form.name.value, enabled: form.enabled.checked, source, last_poll_at: r.last_poll_at ?? null, last_error: null };
      const saved = await attempt(() => (r.id ? api(`/remotes/${enc(r.id)}`, { method: "PUT", json: body }) : api("/remotes", { json: body })), "Remote mailbox saved");
      if (!saved) return false;
      showPage("remotes");
      loadMailboxes();
    });
  const toggle = () => {
    const imap = $("#remote-type").value === "imap";
    document.querySelector(".imap-only").hidden = !imap;
    document.querySelector(".scratchpad-only").hidden = imap;
  };
  $("#remote-type").addEventListener("change", toggle);
  toggle();
}

function confirmDialog(message, detail, action, label = "Delete") {
  openDialog(`<h3>${esc(message)}</h3><p class="lead">${esc(detail)}</p>
    <div class="row">${cancelButton}<button type="submit" class="btn primary destructive" value="ok">${esc(label)}</button></div>`,
    async () => { await action(); });
}

// ---------------------------------------------------------------- actions

/** Downloads an export (.html) without leaving the app, so errors show as a toast. */
async function exportIds(ids, title) {
  toast("Preparing export…");
  await attempt(async () => {
    const response = await request("/export", { json: { ids, title } });
    const disposition = response.headers.get("content-disposition") || "";
    const match = /filename\*=UTF-8''([^;]+)/i.exec(disposition);
    const link = document.createElement("a");
    link.href = URL.createObjectURL(await response.blob());
    link.download = match ? decodeURIComponent(match[1]) : "email.html";
    link.click();
    setTimeout(() => URL.revokeObjectURL(link.href), 10000);
  }, "Export downloaded");
}

async function deleteIds(ids) {
  if (!ids.length) return;
  const index = state.emails.findIndex((e) => e.id === state.selectedId);
  if ((await attempt(() => api("/emails/delete", { json: { ids } }))) === undefined) return;
  ids.forEach((id) => state.checked.delete(id));
  await loadEmails();
  if (ids.includes(state.selectedId)) {
    const next = state.emails[Math.min(index, state.emails.length - 1)];
    if (next) selectEmail(next.id);
    else { state.selectedId = null; state.detail = null; renderDetail(); }
  }
}

document.addEventListener("click", async (event) => {
  const target = event.target.closest("[data-action]");
  if (!target) return;
  const action = target.dataset.action;
  if (action !== "check" && action !== "check-all") event.preventDefault();
  switch (action) {
    case "mailbox":
      state.mailbox = target.dataset.name === "__all__" ? null : target.dataset.name;
      state.checked.clear();
      if (state.page) leavePage();
      renderSidebar();
      await attempt(loadEmails);
      break;
    case "page":
      if ($("#dialog").open) $("#dialog").close();
      showPage(target.dataset.page);
      break;
    case "theme":
      localStorage.setItem(THEME_KEY, target.dataset.theme);
      applyTheme();
      if (SHARE) renderList();
      else renderSidebar();
      break;
    case "select":
      selectEmail(target.dataset.id);
      break;
    case "check":
      event.stopPropagation();
      target.checked ? state.checked.add(target.dataset.id) : state.checked.delete(target.dataset.id);
      renderList();
      break;
    case "check-all":
      state.checked = target.checked ? new Set(state.emails.map((e) => e.id)) : new Set();
      renderList();
      break;
    case "mark-read":
      await attempt(() => api("/emails/read", { json: { ids: [...state.checked], read: true } }));
      await loadEmails();
      break;
    case "delete-checked":
      deleteIds([...state.checked]);
      break;
    case "export-checked": {
      const ids = state.emails.filter((e) => state.checked.has(e.id)).map((e) => e.id);
      exportIds(ids, ids.length === 1 ? null : state.mailbox || `${ids.length} emails`);
      break;
    }
    case "export-one":
      exportIds([state.selectedId], null);
      break;
    case "load-more":
      target.disabled = true;
      await attempt(() => loadEmails(true));
      break;
    case "share-checked":
      shareDialog([...state.checked], state.mailbox || "Shared emails");
      break;
    case "share-mailbox":
      shareDialog(null, state.mailbox || "Shared emails", state.mailbox);
      break;
    case "share-one":
      shareDialog([state.selectedId], state.detail.summary.subject);
      break;
    case "clear-mailbox":
      confirmDialog(`Empty “${state.mailbox}”?`, "All emails in this mailbox will be deleted.", async () => {
        await attempt(() => api(`/mailboxes/${enc(state.mailbox)}/clear`, { method: "POST" }), "Mailbox emptied");
        await loadEmails();
      });
      break;
    case "delete-mailbox":
      confirmDialog(`Delete “${state.mailbox}”?`, "The mailbox and all its emails will be deleted.", async () => {
        await attempt(() => api(`/mailboxes/${enc(state.mailbox)}`, { method: "DELETE" }), "Mailbox deleted");
        state.mailbox = null;
        await Promise.all([loadMailboxes(), loadEmails()]);
      });
      break;
    case "tab":
      state.tab = target.dataset.tab;
      renderDetailHead();
      renderTab();
      break;
    case "toggle-meta":
      state.showMeta = !state.showMeta;
      renderDetailHead();
      break;
    case "forward":
      forwardDialog();
      break;
    case "recheck":
      await attempt(() => api(`/emails/${enc(state.selectedId)}/recheck`, { method: "POST" }), "Checks started");
      break;
    case "delete-one":
      deleteIds([state.selectedId]);
      break;
    case "copy":
      copy(target.dataset.text);
      break;
    case "edit-s3":
      s3Dialog(state.cloud?.configs.find((c) => c.id === target.dataset.id));
      break;
    case "test-s3": {
      const config = state.cloud.configs.find((c) => c.id === target.dataset.id);
      target.disabled = true;
      const checks = await attempt(() => api("/cloud/configs/test", { json: config }));
      target.disabled = false;
      if (checks) openDialog(`<h3>${esc(config.name)}</h3><p class="lead">${checks.every((c) => c.ok) ? "Everything works." : "Some checks failed."}</p>${renderChecks(checks)}
        <div class="row"><button type="submit" class="btn primary" value="ok">Done</button></div>`, async () => {});
      break;
    }
    case "delete-s3":
      confirmDialog("Delete S3 config?", "Existing shares stay online, but you won't be able to renew or delete them from here.", async () => {
        await attempt(() => api(`/cloud/configs/${enc(target.dataset.id)}`, { method: "DELETE" }), "S3 config deleted");
        showPage("cloud");
      });
      break;
    case "vault-create":
      passwordDialog("Set master password", "Synced S3 credentials are encrypted with this password (Argon2id + XChaCha20-Poly1305). It can't be recovered if you forget it.",
        [["password", "Master password"], ["confirm", "Repeat password"]], "Set password",
        (v) => attempt(() => api("/cloud/vault/create", { json: { password: v.password } }), "Master password set"));
      break;
    case "vault-unlock":
      passwordDialog("Unlock vault", "Synced configs stay unlocked until you lock the vault or Scratchpad restarts.", [["password", "Master password"]], "Unlock",
        (v) => attempt(() => api("/cloud/vault/unlock", { json: v }), "Vault unlocked"));
      break;
    case "vault-password":
      passwordDialog("Change master password", "The vault is re-encrypted and synced with the new password.",
        [["current", "Current password"], ["new", "New password"], ["confirm", "Repeat new password"]], "Change password",
        (v) => attempt(() => api("/cloud/vault/password", { json: { current: v.current, new: v.new } }), "Master password changed"));
      break;
    case "vault-restore":
      passwordDialog("Restore from cloud", "Replaces the synced configs on this device with the ones synced from another device.", [["password", "Master password"]], "Restore",
        (v) => attempt(() => api("/cloud/vault/restore", { json: v }), "Synced configs restored"));
      break;
    case "vault-lock":
      await attempt(() => api("/cloud/vault/lock", { method: "POST" }), "Vault locked");
      showPage("cloud");
      break;
    case "vault-sync":
      await attempt(() => api("/cloud/vault/sync", { method: "POST" }), "Vault synced");
      showPage("cloud");
      break;
    case "vault-reset":
      confirmDialog("Reset the vault?", "The master password and all synced S3 configs are deleted from this device and from the cloud. Configs on “This device” stay.", async () => {
        await attempt(() => api("/cloud/vault", { method: "DELETE" }), "Vault reset");
        showPage("cloud");
      });
      break;
    case "renew-cloud-share": {
      const id = target.dataset.id;
      openDialog(`<h3>New link</h3><p class="lead">Creates a new presigned link. Links you already sent keep working until they expire.</p>
        <label class="lbl" for="renew-expires">Expires after</label><select id="renew-expires" name="expires">${EXPIRY.map(([v, l]) => `<option value="${v}" ${v === 86400 ? "selected" : ""}>${l}</option>`).join("")}</select>
        ${dialogButtons(`${icon("link", "sm")}Create & copy link`)}`,
        async (form) => {
          const share = await attempt(() => api(`/cloud/shares/${enc(id)}/renew`, { json: { expires_in_secs: Number(form.expires.value) } }));
          if (!share) return false;
          await copy(share.url);
          showPage("shares");
        });
      break;
    }
    case "delete-cloud-share": {
      const id = target.dataset.id;
      confirmDialog("Delete share?", "The uploaded file is deleted from the bucket, so the link stops working.", async () => {
        try {
          await api(`/cloud/shares/${enc(id)}`, { method: "DELETE" });
          toast("Share deleted");
          showPage("shares");
        } catch (error) {
          setTimeout(() => confirmDialog("Couldn't delete the uploaded file", `${error.message}. Remove the share from this list anyway? The file stays in the bucket.`, async () => {
            await attempt(() => api(`/cloud/shares/${enc(id)}?keep_object=true`, { method: "DELETE" }), "Removed from list");
            showPage("shares");
          }, "Remove from list"), 0);
        }
      });
      break;
    }
    case "delete-share":
      await attempt(() => api(`/shares/${enc(target.dataset.token)}`, { method: "DELETE" }), "Share revoked");
      showPage("shares");
      break;
    case "edit-remote":
      remoteDialog(state.remotes.find((r) => r.id === target.dataset.id));
      break;
    case "delete-remote":
      confirmDialog("Delete remote mailbox?", "The remote mailbox and its imported emails will be deleted.", async () => {
        await attempt(() => api(`/remotes/${enc(target.dataset.id)}?delete_emails=true`, { method: "DELETE" }), "Remote mailbox deleted");
        showPage("remotes");
        loadMailboxes();
      });
      break;
    case "poll-remotes":
      await attempt(() => api("/remotes/poll", { method: "POST" }), "Checking remote mailboxes…");
      setTimeout(() => state.page === "remotes" && showPage("remotes"), 3000);
      break;
    case "test-email": {
      const mailbox = $("#test-mailbox").value || "Test";
      await attempt(() => api("/test-email", { json: { mailbox } }), `Test email sent to “${mailbox}”`);
      break;
    }
  }
});

document.addEventListener("submit", async (event) => {
  if (event.target.id !== "settings-form") return;
  event.preventDefault();
  const settings = readForm(event.target, state.settings);
  const saved = await attempt(() => api("/settings", { method: "PUT", json: settings }), "Settings saved");
  if (saved) {
    state.settings = saved;
    if (state.page === "settings") $("#detail").innerHTML = renderSettings();
  }
  setTimeout(() => loadStatus().catch(() => {}), 500);
});

let searchTimer;
document.addEventListener("input", (event) => {
  if (event.target.type === "password") event.target.dataset.dirty = "1";
  if (event.target.id !== "search") return;
  clearTimeout(searchTimer);
  searchTimer = setTimeout(async () => {
    state.search = event.target.value;
    await attempt(loadEmails);
    const input = $("#search");
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
  }, 180);
});

// Keyboard: arrows/j/k navigate, Backspace/Delete removes the selected email, / searches.
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && event.target.id === "search") { event.target.blur(); return; }
  if (event.target.closest("input, textarea, select, dialog") || event.metaKey || event.ctrlKey || event.altKey) return;
  if ((event.key === "Enter" || event.key === " ") && event.target.matches(".email-row")) {
    event.preventDefault();
    selectEmail(event.target.dataset.id);
    return;
  }
  if (state.page) return;
  const index = state.emails.findIndex((e) => e.id === state.selectedId);
  if (event.key === "/" && !SHARE) {
    event.preventDefault();
    $("#search")?.focus();
  } else if (event.key === "ArrowDown" || event.key === "j") {
    const next = state.emails[index + 1] || (index === -1 ? state.emails[0] : null);
    if (next) { event.preventDefault(); selectEmail(next.id); scrollToActive(); }
  } else if (event.key === "ArrowUp" || event.key === "k") {
    const prev = state.emails[index - 1];
    if (prev) { event.preventDefault(); selectEmail(prev.id); scrollToActive(); }
  } else if ((event.key === "Backspace" || event.key === "Delete") && state.selectedId && !SHARE) {
    event.preventDefault();
    deleteIds([state.selectedId]);
  }
});

function scrollToActive() {
  requestAnimationFrame(() => {
    const row = $(".email-row.active");
    row?.scrollIntoView({ block: "nearest" });
    row?.focus({ preventScroll: true });
  });
}

// ---------------------------------------------------------------- live updates

function notify(email) {
  if (!state.settings?.notifications || !("Notification" in window)) return;
  if (Notification.permission === "granted" && document.hidden) {
    const n = new Notification(email.subject, { body: `${email.from}\n${email.mailbox}`, icon: "/assets/icon.svg" });
    n.onclick = () => {
      window.focus();
      state.mailbox = email.mailbox;
      loadEmails().then(() => selectEmail(email.id));
    };
  }
}

const reloadEmails = () => debounce("list", () => loadEmails().catch(() => {}), 250);

function connectEvents(delay = 1000) {
  const source = new EventSource(`${API}/events`);
  source.onmessage = async (message) => {
    const event = JSON.parse(message.data);
    switch (event.type) {
      case "email_received":
        notify(event.email);
        // Bursts of mail (test suites) reload the list once.
        if (state.mailbox === null || state.mailbox === event.email.mailbox) reloadEmails();
        debounce("mailboxes", () => loadMailboxes().catch(() => {}), 250);
        break;
      case "email_updated":
        if (event.id === state.selectedId) loadDetail(event.id).catch(() => {});
        if (state.emails.some((e) => e.id === event.id)) reloadEmails();
        break;
      case "emails_deleted":
        if (event.ids.some((id) => state.emails.some((e) => e.id === id))) reloadEmails();
        break;
      case "mailboxes_changed":
        debounce("mailboxes", () => loadMailboxes().catch(() => {}), 200);
        break;
      case "shares_changed":
        if (state.page === "shares") showPage("shares");
        break;
      case "cloud_changed":
        if (state.page === "cloud" || state.page === "shares") debounce("cloud", () => showPage(state.page), 200);
        break;
      case "remotes_changed":
        if (state.page === "remotes") showPage("remotes");
        break;
      case "settings_changed":
        state.settings = (await api("/settings").catch(() => null)) || state.settings;
        break;
      case "smtp_status":
        loadStatus().catch(() => {});
        break;
      case "resync":
        refreshAll();
        break;
    }
  };
  source.onopen = () => { delay = 1000; };
  source.onerror = () => {
    // EventSource retries by itself while the connection drops, but gives up for good
    // when a response isn't an event stream (e.g. 401 after the password changed, or a
    // proxy error page). Reconnect with backoff then. Either way, refresh once back.
    source.onopen = () => { delay = 1000; refreshAll(); };
    if (source.readyState === EventSource.CLOSED) {
      setTimeout(() => connectEvents(Math.min(delay * 2, 30000)), delay);
    }
  };
}

function refreshAll() {
  loadStatus().catch(() => {});
  loadMailboxes().catch(() => {});
  loadEmails().catch(() => {});
}

const timers = {};
function debounce(key, fn, ms) {
  clearTimeout(timers[key]);
  timers[key] = setTimeout(fn, ms);
}

// ---------------------------------------------------------------- boot

async function boot() {
  if (SHARE) {
    document.getElementById("app").classList.add("share-mode");
    document.title = "Shared emails · Scratchpad";
    await attempt(loadEmails);
    renderDetail();
    if (state.emails[0]) selectEmail(state.emails[0].id);
    // Inbox shares are live: pick up emails that arrived since the page was opened.
    if (state.shareMailbox) setInterval(() => { if (!document.hidden) loadEmails().catch(() => {}); }, 10000);
    return;
  }
  renderDetail();
  await attempt(async () => {
    [state.settings] = await Promise.all([api("/settings"), loadStatus(), loadMailboxes(), loadEmails()]);
  });
  renderDetail();
  connectEvents();
  if (state.settings?.notifications && "Notification" in window && Notification.permission === "default") {
    document.addEventListener("click", () => Notification.requestPermission(), { once: true });
  }
}

boot();
