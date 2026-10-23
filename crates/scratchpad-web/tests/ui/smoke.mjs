// Browser smoke test for the Scratchpad web interface and the exported .html viewer.
//
// It syntax-checks every JS asset (`node --check`), starts a headless Scratchpad (or uses a
// running one), delivers test emails over SMTP, then drives headless Chrome through the
// DevTools protocol: mailbox list, search, selection and bulk actions, keyboard navigation,
// every detail tab and device preview, export, sharing (server link and share page),
// the Shares, Cloud (S3 dialog, vault), Settings, Remote mailboxes and Setup pages, live
// updates over SSE, themes, and an exported file opened from file:// (which must make no
// network requests). It fails on any uncaught exception, console error or CSP violation.
//
// No npm dependencies (Node 22+: global fetch and WebSocket).
//
//   # build the headless binary, then let the script start it on ports 3601/9601:
//   cargo build -p scratchpad --no-default-features
//   SCRATCHPAD_BIN=target/debug/scratchpad node crates/scratchpad-web/tests/ui/smoke.mjs
//
//   # or test an instance that is already running (it must accept SMTP on SCRATCHPAD_SMTP):
//   SCRATCHPAD_URL=http://127.0.0.1:9601 SCRATCHPAD_SMTP=127.0.0.1:3601 node crates/scratchpad-web/tests/ui/smoke.mjs
//
// Environment:
//   CHROME           Chrome/Chromium binary (default: macOS Google Chrome, else `google-chrome`)
//   SCRATCHPAD_BIN   scratchpad binary to start with a temporary data dir (optional)
//   SCRATCHPAD_URL   web interface URL (default http://127.0.0.1:9601)
//   SCRATCHPAD_SMTP  SMTP host:port (default 127.0.0.1:3601)
//   SCREENSHOTS      directory for screenshots of key screens at 1440 and 390 px, dark and light (optional)
//   CDP_PORT         DevTools port (default 9339)

import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { createConnection } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ASSETS = resolve(HERE, "../../assets");
const BASE = (process.env.SCRATCHPAD_URL || "http://127.0.0.1:9601").replace(/\/$/, "");
const [SMTP_HOST, SMTP_PORT] = (process.env.SCRATCHPAD_SMTP || "127.0.0.1:3601").split(":");
const CHROME = process.env.CHROME
  || (process.platform === "darwin" ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" : "google-chrome");
const CDP_PORT = Number(process.env.CDP_PORT || 9339);
const SHOTS = process.env.SCREENSHOTS ? resolve(process.env.SCREENSHOTS) : null;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const temp = mkdtempSync(join(tmpdir(), "scratchpad-smoke-"));
const children = [];
let failures = 0;

function fail(message) {
  failures++;
  console.error(`  ✗ ${message}`);
}

function check(condition, message) {
  if (condition) console.log(`  ✓ ${message}`);
  else fail(message);
}

// ------------------------------------------------------------------ JS syntax

console.log("Syntax");
for (const file of ["app.js", "boot.js", "devices.js", "export/export.js"]) {
  try {
    execFileSync(process.execPath, ["--check", join(ASSETS, file)], { stdio: "pipe" });
    console.log(`  ✓ node --check ${file}`);
  } catch (error) {
    fail(`node --check ${file}: ${error.stderr}`);
  }
}
const extra = readdirSync(ASSETS).filter((f) => f.endsWith(".js") && !["app.js", "boot.js", "devices.js"].includes(f));
check(!extra.length, `no unchecked JS assets (${extra.join(", ") || "none"})`);

// ------------------------------------------------------------------ server

async function waitFor(fn, what, timeout = 15000) {
  const end = Date.now() + timeout;
  for (;;) {
    try {
      const value = await fn();
      if (value) return value;
    } catch (_) {}
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await sleep(100);
  }
}

if (process.env.SCRATCHPAD_BIN) {
  const web = new URL(BASE);
  const scratchpad = spawn(resolve(process.env.SCRATCHPAD_BIN), ["--headless", "--data-dir", join(temp, "data"),
    "--smtp-port", SMTP_PORT, "--web-port", web.port || "80", "--log", "warn"], { stdio: ["ignore", "ignore", "inherit"] });
  children.push(scratchpad);
  scratchpad.on("exit", (code) => code && console.error(`scratchpad exited with ${code}`));
}
await waitFor(async () => (await fetch(`${BASE}/healthz`)).ok, "the web interface");
const api = async (path, init = {}) => {
  const response = await fetch(`${BASE}/api${path}`, {
    ...init,
    headers: init.body ? { "Content-Type": "application/json" } : {},
  });
  if (!response.ok) throw new Error(`${path}: ${response.status} ${await response.text()}`);
  return response.status === 204 || response.status === 202 ? null : response.json();
};
// Start from a clean slate when testing an existing instance.
for (const mailbox of await api("/mailboxes")) await api(`/mailboxes/${encodeURIComponent(mailbox.name)}`, { method: "DELETE" });
await api("/settings").then((s) => api("/settings", { method: "PUT", body: JSON.stringify({ ...s, link_checker: true, link_checker_timeout_secs: 3, spam_check: true, web_password: null, forward: { ...s.forward, password: null } }) }));

// ------------------------------------------------------------------ SMTP

/** Delivers `raw` to `mailbox` (the SMTP username) with a minimal SMTP client. */
function sendMail(mailbox, raw) {
  return new Promise((resolveSend, reject) => {
    const socket = createConnection(Number(SMTP_PORT), SMTP_HOST);
    const auth = Buffer.from(`\0${mailbox}\0x`).toString("base64");
    const script = ["EHLO smoke.test", `AUTH PLAIN ${auth}`, "MAIL FROM:<app@example.com>", "RCPT TO:<dev@example.com>",
      "RCPT TO:<hidden@example.com>", "DATA", raw.replace(/\r?\n/g, "\r\n").replace(/^\./gm, "..") + "\r\n.", "QUIT"];
    let buffer = "";
    socket.setEncoding("utf8");
    socket.on("data", (data) => {
      buffer += data;
      // Wait for the last line of a (multi-line) reply: "250 ..." rather than "250-...".
      const lines = buffer.split("\r\n").filter(Boolean);
      const last = lines[lines.length - 1];
      if (!buffer.endsWith("\r\n") || !/^\d{3} /.test(last)) return;
      buffer = "";
      if (/^[45]/.test(last)) return reject(new Error(`SMTP: ${last}`));
      const next = script.shift();
      if (next === undefined) return socket.end();
      socket.write(next + "\r\n");
    });
    socket.on("close", () => resolveSend());
    socket.on("error", reject);
  });
}

const b64 = (text) => Buffer.from(text).toString("base64");
const PIXEL = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
const RICH = `From: "Ada Lovelace" <ada@example.com>
To: Dev Team <dev@example.com>
Cc: qa@example.com
Subject: Welcome <img src=x onerror="window.__xss=1"> to Scratchpad
Message-ID: <rich@smoke.test>
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary=mixed

--mixed
Content-Type: multipart/alternative; boundary=alt

--alt
Content-Type: text/plain; charset=utf-8

Welcome! Visit https://example.invalid/start

--alt
Content-Type: multipart/related; boundary=rel

--rel
Content-Type: text/html; charset=utf-8

<!doctype html><html><head><style>@media (max-width: 500px) { .wide { display: none } }</style></head>
<body style="font-family:sans-serif;margin:0;padding:24px;background:#f6f6ff">
<h1>Welcome aboard</h1><p class="wide">Only on wide screens.</p>
<img src="cid:logo@smoke" alt="logo" width="40" height="40">
<p><a href="https://example.invalid/start">Get started</a> · <a href="javascript:window.__xss=2">Evil link</a>
 · <a href="view-data">Relative</a></p>
<script>window.__xss = 3</script>
<form action="/api/mailboxes/Default/clear" method="post"><button>Clear</button></form>
${"<p>Long content line to make the email tall enough to scroll.</p>\n".repeat(40)}
</body></html>
--rel
Content-Type: image/png
Content-ID: <logo@smoke>
Content-Transfer-Encoding: base64

${PIXEL}
--rel--

--alt--

--mixed
Content-Type: text/html; name="notes.html"
Content-Disposition: attachment; filename*=UTF-8''r%C3%A9sum%C3%A9%22%3B.html

<script>window.top.__xss = 4</script><p>attachment</p>
--mixed
Content-Type: application/pdf; name="invoice.pdf"
Content-Disposition: attachment; filename="invoice.pdf"
Content-Transfer-Encoding: base64

JVBERi0xLjQKJcOkw7zDtsOfCjEgMCBvYmoKPDwvVHlwZS9DYXRhbG9nPj4KZW5kb2JqCnRyYWlsZXIKPDwvUm9vdCAxIDAgUj4+CiUlRU9G
--mixed--
`;
const PLAIN = (n) => `From: Grace <grace@example.com>
To: dev@example.com
Subject: Plain message ${n}
Date: Mon, 12 Oct 2026 10:0${n}:00 +0000

Just text, number ${n}.
`;
const VIEW = `From: Laravel <app@example.com>
To: dev@example.com
Subject: Order shipped
X-HELO-View: ${b64("<x-mail::message>Order {{ $order->id }}</x-mail::message>")}
X-HELO-View-File: ${b64("resources/views/emails/order.blade.php")}
X-HELO-View-Data: ${b64("<pre class=sf-dump>array:1 [ \"order\" => 42 ]</pre><script>document.body.dataset.ran = 1</script>")}
Content-Type: text/html

<p>Your order <b>#42</b> has shipped.</p>
`;

console.log("Seed");
await sendMail("Smoke", RICH);
for (let n = 1; n <= 3; n++) await sendMail("Smoke", PLAIN(n));
await sendMail("Laravel", VIEW);
await waitFor(async () => (await api("/emails")).length === 5, "5 emails");
console.log("  ✓ 5 emails delivered over SMTP");

// ------------------------------------------------------------------ Chrome

const chrome = spawn(CHROME, ["--headless=new", `--remote-debugging-port=${CDP_PORT}`, `--user-data-dir=${join(temp, "chrome")}`,
  "--no-first-run", "--no-default-browser-check", "--hide-scrollbars", "--force-color-profile=srgb", "--disable-gpu",
  "--no-sandbox", "--allow-file-access-from-files", "about:blank"], { stdio: "ignore" });
children.push(chrome);

function cleanup() {
  for (const child of children) child.kill();
  try { rmSync(temp, { recursive: true, force: true }); } catch (_) {}
}
process.on("exit", cleanup);
process.on("SIGINT", () => process.exit(130));

const targets = await waitFor(async () => {
  const list = await (await fetch(`http://127.0.0.1:${CDP_PORT}/json/list`)).json();
  return list.some((t) => t.type === "page") && list;
}, "Chrome");
const ws = new WebSocket(targets.find((t) => t.type === "page").webSocketDebuggerUrl);
await new Promise((r, j) => { ws.addEventListener("open", r); ws.addEventListener("error", j); });

let seq = 0;
const pending = new Map();
const listeners = [];
ws.addEventListener("message", (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
  } else if (msg.method) {
    for (const listener of listeners) listener(msg);
  }
});
const send = (method, params = {}) => new Promise((ok, no) => {
  const n = ++seq;
  pending.set(n, (msg) => (msg.error ? no(new Error(`${method}: ${JSON.stringify(msg.error)}`)) : ok(msg.result)));
  ws.send(JSON.stringify({ id: n, method, params }));
});

const problems = [];
const requests = [];
listeners.push(({ method, params }) => {
  if (method === "Runtime.exceptionThrown") {
    const d = params.exceptionDetails;
    problems.push(`exception: ${d.exception?.description || d.text} (${d.url || ""}:${d.lineNumber})`);
  } else if (method === "Runtime.consoleAPICalled" && ["error", "assert"].includes(params.type)) {
    problems.push(`console.${params.type}: ${params.args.map((a) => a.value ?? a.description).join(" ")}`);
  } else if (method === "Log.entryAdded" && params.entry.level === "error") {
    // Expected API errors (e.g. a 400 the UI shows as a toast) are logged by the network
    // stack; everything else (CSP violations, failed scripts) is a problem.
    const e = params.entry;
    // The sandbox blocking an email's own <script> is expected.
    const expected = (e.source === "network" && /status of 4\d\d/.test(e.text))
      || /Blocked script execution in .* the document's frame is sandboxed/.test(e.text);
    if (!expected) problems.push(`${e.source}: ${e.text} ${e.url || ""}`);
  } else if (method === "Page.javascriptDialogOpening") {
    // `copy()` falls back to prompt() when the clipboard is unavailable.
    send("Page.handleJavaScriptDialog", { accept: true }).catch(() => {});
  } else if (method === "Network.requestWillBeSent") {
    requests.push(params.request.url);
  }
});
await send("Page.enable");
await send("Runtime.enable");
await send("Log.enable");
await send("Network.enable");
await send("Browser.setDownloadBehavior", { behavior: "allow", downloadPath: join(temp, "downloads") }).catch(() => {});
await send("Browser.grantPermissions", { origin: BASE, permissions: ["clipboardReadWrite", "clipboardSanitizedWrite", "notifications"] }).catch(() => {});

async function evaluate(expression) {
  const r = await send("Runtime.evaluate", { expression: `(async () => { ${expression} })()`, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(`${r.exceptionDetails.exception?.description || r.exceptionDetails.text}\n  in: ${expression.slice(0, 200)}`);
  return r.result.value;
}

async function until(expression, what, timeout = 5000) {
  try {
    return await waitFor(() => evaluate(`return ${expression}`), what, timeout);
  } catch (error) {
    fail(error.message);
    return null;
  }
}

async function viewport(width, height, dark) {
  await send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: SHOTS ? 2 : 1, mobile: width < 600 });
  await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: dark ? "dark" : "light" }] });
}

async function navigate(url) {
  await send("Page.navigate", { url });
  await sleep(300);
  await until(`document.readyState === "complete"`, `load ${url}`);
}

async function shot(name) {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  await sleep(400);
  const { data } = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(SHOTS, `${name}.png`), Buffer.from(data, "base64"));
}

/** Runs a step; reports and continues on failure so one run shows every problem. */
async function step(name, fn) {
  console.log(name);
  const before = problems.length;
  try {
    await fn();
  } catch (error) {
    fail(error.message);
  }
  await sleep(150);
  for (const problem of problems.slice(before)) fail(problem);
}

const click = (selector) => `const el = document.querySelector(${JSON.stringify(selector)}); if (!el) throw new Error("no " + ${JSON.stringify(selector)}); el.click();`;
const count = (selector) => `document.querySelectorAll(${JSON.stringify(selector)}).length`;
const toastText = `(document.querySelector("#toast:not([hidden])")?.textContent || "")`;
const key = async (key, code = key) => {
  for (const type of ["keyDown", "keyUp"]) {
    const text = type === "keyDown" ? { Enter: "\r" }[key] ?? (key.length === 1 ? key : undefined) : undefined;
    await send("Input.dispatchKeyEvent", { type: text ? type : type === "keyDown" ? "rawKeyDown" : type, text, key, code, windowsVirtualKeyCode: { ArrowDown: 40, ArrowUp: 38, Enter: 13, Backspace: 8, Escape: 27 }[key] || key.toUpperCase().charCodeAt(0) });
  }
};

// ------------------------------------------------------------------ app

await viewport(1440, 900, true);

await step("Mailbox list", async () => {
  await navigate(`${BASE}/`);
  await until(`${count(".email-row")} === 5`, "5 rows in All emails");
  check(await evaluate(`return document.querySelector(".list-head h2").textContent.includes("5")`), "count shown");
  check(await evaluate(`return ${count(".side-item[data-action=mailbox]")} === 3`), "All + 2 mailboxes in the sidebar");
  await evaluate(click(`.side-item[data-name="Smoke"]`));
  await until(`${count(".email-row")} === 4`, "4 rows in Smoke");
  check(await evaluate(`return !!document.querySelector("[data-action=clear-mailbox]") && !!document.querySelector("[data-action=delete-mailbox]")`), "mailbox actions");
  await evaluate(click(`.side-item[data-name="__all__"]`));
  await until(`${count(".email-row")} === 5`, "back to all");
  check(await evaluate(`return [...document.querySelectorAll(".email-row .subject")].some((s) => s.textContent.includes("<img src=x"))`), "subject markup shown as text");
});

await step("Search", async () => {
  await evaluate(`const s = document.querySelector("#search"); s.focus(); s.value = "Plain message 2"; s.dispatchEvent(new Event("input", { bubbles: true }));`);
  await until(`${count(".email-row")} === 1`, "search narrows to 1");
  await evaluate(`const s = document.querySelector("#search"); s.value = "zzzz-nothing"; s.dispatchEvent(new Event("input", { bubbles: true }));`);
  await until(`!!document.querySelector(".emails .empty")`, "no-results state");
  await evaluate(`const s = document.querySelector("#search"); s.value = ""; s.dispatchEvent(new Event("input", { bubbles: true })); s.blur();`);
  await until(`${count(".email-row")} === 5`, "search cleared");
});

await step("Detail tabs and device previews", async () => {
  await evaluate(`[...document.querySelectorAll(".email-row")].find((r) => r.textContent.includes("Ada Lovelace")).click()`);
  await until(`!!document.querySelector(".detail-head h1")`, "detail");
  await until(`!!document.querySelector(".dv-stage iframe")`, "preview iframe");
  await shot("app-1440-dark");
  for (const device of ["desktop", "laptop", "tablet", "mobile", "all", "fit"]) {
    await evaluate(click(`.dv-switch [data-device="${device}"]`));
    await until(`document.querySelector('.dv-switch [data-device="${device}"]').classList.contains("on")`, `device ${device}`);
    if (device === "mobile") {
      await sleep(300);
      check(await evaluate(`const f = document.querySelector(".dv-device"); return f && f.getBoundingClientRect().width > 50 && f.getBoundingClientRect().width <= 391`), "mobile frame scaled to fit");
      await evaluate(click(".dv-rotate"));
      await until(`document.querySelector(".dv-device")?.dataset.w === "844"`, "rotated to landscape");
      await evaluate(click(".dv-rotate"));
    }
    if (device === "all") {
      await until(`${count(".dv-device")} === 3`, "three devices side by side");
      await shot("app-devices-all-1440-dark");
    }
  }
  // The email can't run scripts, and the sandbox reaches the frame.
  await until(`document.querySelector(".dv-stage iframe")?.contentDocument?.querySelector("img")?.complete`, "preview loaded");
  const frame = await evaluate(`const f = document.querySelector(".dv-stage iframe"); return { sandbox: f.getAttribute("sandbox"), img: f.contentDocument.querySelector("img")?.naturalWidth, text: f.contentDocument.body.textContent.includes("Welcome aboard") }`);
  check(!frame.sandbox.includes("allow-scripts") && frame.text, "preview is sandboxed and rendered");
  check(frame.img === 1, "inline cid: image loads");
  const tabs = await evaluate(`return [...document.querySelectorAll(".tabs .tab")].map((t) => t.dataset.tab)`);
  check(tabs.join() === "html,text,source,raw,headers,attachments,links,spam", `tabs: ${tabs.join(", ")}`);
  for (const tab of tabs) {
    await evaluate(click(`.tab[data-tab="${tab}"]`));
    await until(`document.querySelector('.tab[data-tab="${tab}"]').getAttribute("aria-selected") === "true"`, `tab ${tab}`);
    if (tab === "raw") await until(`document.querySelector("#tab-body pre")?.textContent.includes("Message-ID")`, "raw source");
    if (tab === "attachments") {
      check(await evaluate(`return ${count("#tab-body .card")} === 3`), "3 attachments listed");
      check(await evaluate(`return [...document.querySelectorAll("#tab-body .card .title")].some((t) => t.textContent.includes("résumé"))`), "UTF-8 attachment name");
    }
    if (tab === "links") {
      await until(`!!document.querySelector("#tab-body table")`, "links checked", 10000);
      const hrefs = await evaluate(`return [...document.querySelectorAll("#tab-body a")].map((a) => a.getAttribute("href"))`);
      check(hrefs.length >= 1 && hrefs.every((h) => /^https?:/.test(h)), `only http(s) links are clickable (${hrefs.join(" ")})`);
      check(await evaluate(`return document.querySelector("#tab-body").textContent.includes("javascript:")`), "javascript: link listed as text");
    }
    if (tab === "spam") await until(`!!document.querySelector(".score-card") || !!document.querySelector("#tab-body .empty")`, "spam report");
  }
  await evaluate(click("[data-action=toggle-meta]"));
  check(await evaluate(`return document.querySelector(".meta-grid")?.textContent.includes("hidden@example.com")`), "details show Bcc from the envelope");
  await evaluate(click(`.tab[data-tab="html"]`));
  check(await evaluate(`return window.__xss === undefined`), "no script from the email ran in the app");
});

await step("View inspector", async () => {
  await evaluate(`[...document.querySelectorAll(".email-row")].find((r) => r.textContent.includes("Order shipped")).click()`);
  await until(`!!document.querySelector('.tab[data-tab="inspector"]')`, "inspector tab");
  await evaluate(click(`.tab[data-tab="inspector"]`));
  await until(`!!document.querySelector(".inspector iframe")`, "view data frame");
  check(await evaluate(`return document.querySelector(".inspector").textContent.includes("order.blade.php")`), "view file shown");
  check(await evaluate(`return document.querySelector(".inspector iframe").getAttribute("sandbox") === "allow-scripts"`), "view data sandboxed");
});

await step("Keyboard navigation", async () => {
  await evaluate(`document.activeElement?.blur(); document.body.focus()`);
  const first = await evaluate(`return document.querySelector(".email-row.active").dataset.id`);
  await key("j", "KeyJ");
  await until(`document.querySelector(".email-row.active")?.dataset.id !== ${JSON.stringify(first)}`, "j selects the next email");
  await key("k", "KeyK");
  await until(`document.querySelector(".email-row.active")?.dataset.id === ${JSON.stringify(first)}`, "k goes back");
  await key("/", "Slash");
  check(await evaluate(`return document.activeElement.id === "search"`), "/ focuses search");
  await key("Escape");
  check(await evaluate(`return document.activeElement.id !== "search"`), "Escape leaves search");
});

await step("Selection, bulk actions and export", async () => {
  await evaluate(`document.querySelectorAll(".email-row .cb")[1].click()`);
  await until(`!!document.querySelector(".bulk")`, "bulk bar");
  await evaluate(click(".bulk [data-action=check-all]"));
  await until(`document.querySelector(".bulk .grow")?.textContent.startsWith("5")`, "select all");
  await evaluate(click(".bulk [data-action=mark-read]"));
  await until(`${count(".email-row.unread")} === 0`, "marked read");
  await evaluate(click(".bulk [data-action=export-checked]"));
  await until(`${toastText}.includes("Export downloaded")`, "export downloaded", 15000);
  await evaluate(click(".bulk [data-action=check-all]"));
  await until(`!document.querySelector(".bulk")`, "selection cleared");
  await evaluate(click(".email-row"));
  await until(`!!document.querySelector("[data-action=export-one]")`, "toolbar export");
  await evaluate(click("[data-action=export-one]"));
  await until(`${toastText}.includes("Export downloaded")`, "single export downloaded", 15000);
});

let shareUrl = null;
await step("Share dialog and share page", async () => {
  await evaluate(click("[data-action=share-one]"));
  await until(`document.querySelector("#dialog").open`, "share dialog");
  await shot("app-share-dialog-1440-dark");
  check(await evaluate(`return document.querySelector("#dialog .cloud-only").hidden`), "no S3 config: server link only");
  // Enter in the title field submits (not Cancel).
  await evaluate(`document.querySelector("#share-title").focus()`);
  await key("Enter");
  await until(`!document.querySelector("#dialog").open`, "Enter creates the link");
  await sleep(300);
  const shares = await api("/shares");
  check(shares.length === 1, "share created");
  shareUrl = shares[0]?.url;

  await evaluate(click(`.side-item[data-name="Smoke"]`));
  await until(`!!document.querySelector("[data-action=share-mailbox]")`, "inbox share button");
  await evaluate(click("[data-action=share-mailbox]"));
  await until(`document.querySelector("#dialog").open`, "inbox share dialog");
  check(await evaluate(`return document.querySelector("#dialog .pick").hidden`), "whole inbox hides the email picker");
  await evaluate(`document.querySelector('input[name=scope][value=emails]').click()`);
  check(await evaluate(`return !document.querySelector("#dialog .pick").hidden`), "only-these-emails shows the picker");
  await evaluate(`document.querySelector('input[name=scope][value=inbox]').click()`);
  await evaluate(click(`#dialog button[type=submit]`));
  await until(`!document.querySelector("#dialog").open`, "inbox link created");
  await until(`${toastText}.length > 0`, "copied toast");

  await evaluate(click(`[data-action=page][data-page=shares]`));
  await until(`${count(".page .card")} === 2`, "Shares page lists both links");
  await shot("app-shares-1440-dark");

  // The share page itself (no sidebar, read only).
  await navigate(shareUrl);
  await until(`!!document.querySelector(".detail-head h1")`, "share page opens the email");
  check(await evaluate(`return document.querySelector(".app").classList.contains("share-mode") && !document.querySelector("[data-action=delete-one]") && !document.querySelector("#search")`), "share page is read-only");
  await until(`!!document.querySelector(".dv-stage iframe")`, "share preview");
  for (const tab of ["text", "raw", "headers", "attachments", "html"]) {
    await evaluate(click(`.tab[data-tab="${tab}"]`));
    await sleep(150);
  }
  await evaluate(click(`.list [data-theme="light"]`));
  check(await evaluate(`return document.documentElement.dataset.theme === "light"`), "theme switch on share page");
  await shot("share-page-1440-light");
  await evaluate(click(`.list [data-theme="system"]`));
  check(await evaluate(`return window.__xss === undefined`), "no script from the email ran on the share page");
});

await step("Shares page actions", async () => {
  await navigate(`${BASE}/`);
  await until(`${count(".email-row")} === 5`, "app reloaded");
  await evaluate(click(`[data-action=page][data-page=shares]`));
  await until(`${count(".page .card")} === 2`, "two shares");
  await evaluate(click(`[data-action=delete-share]`));
  await until(`${count(".page .card")} === 1`, "revoked one");
});

await step("Cloud page: S3 dialog and vault", async () => {
  await evaluate(click(`[data-action=page][data-page=cloud]`));
  await until(`!!document.querySelector("[data-action=vault-create]")`, "cloud page");
  await shot("app-cloud-1440-dark");
  await evaluate(click(`[data-action=edit-s3][data-id=""]`));
  await until(`document.querySelector("#dialog").open`, "S3 dialog");
  check(await evaluate(`return [...document.querySelectorAll("#dialog .grid2 > label")].every((l) => l.htmlFor && document.getElementById(l.htmlFor))`), "every S3 field has a label");
  await evaluate(`const f = document.querySelector("#dialog-form"); f.name.value = "Local"; f.endpoint.value = "http://127.0.0.1:9"; f.bucket.value = "b"; f.access_key.value = "AK"; f.secret_key.value = "SK";`);
  await evaluate(click(`#dialog button[value=test]`));
  await until(`!!document.querySelector("#s3-checks .checks .bad")`, "test reports failed checks", 20000);
  check(await evaluate(`return document.querySelector("#dialog").open`), "dialog stays open after testing");
  await evaluate(click(`#dialog button[value=save], #dialog button[type=submit]`));
  await until(`${count(".page .card .title")} >= 2`, "config saved and listed", 8000);
  check(!JSON.stringify(await api("/cloud")).includes('"SK"'), "secret not returned");
  await evaluate(click(`[data-action=edit-s3]:not([data-id=""])`));
  await until(`document.querySelector("#dialog").open`, "edit dialog");
  check(await evaluate(`return document.querySelector("#dialog [name=secret_key]").placeholder === "Unchanged"`), "secret is write-only in the dialog");
  await evaluate(click(`#dialog button[value=cancel]`));
  await until(`!document.querySelector("#dialog").open`, "Cancel closes");

  await evaluate(click(`[data-action=vault-create]`));
  await until(`document.querySelector("#dialog").open`, "vault dialog");
  await evaluate(`const f = document.querySelector("#dialog-form"); f.password.value = "correct horse"; f.confirm.value = "correct horse";`);
  await evaluate(click(`#dialog button[type=submit]`));
  await until(`!!document.querySelector("[data-action=vault-lock]")`, "vault created and unlocked", 15000);
  await evaluate(click(`[data-action=vault-sync]`));
  await until(`${toastText}.includes("synced")`, "vault synced", 10000);
  await evaluate(click(`[data-action=vault-lock]`));
  await until(`!!document.querySelector("[data-action=vault-unlock]")`, "vault locked");
  await evaluate(click(`[data-action=vault-unlock]`));
  await until(`document.querySelector("#dialog").open`, "unlock dialog");
  await evaluate(`document.querySelector("#dialog-form").password.value = "wrong";`);
  await evaluate(click(`#dialog button[type=submit]`));
  await until(`document.querySelector("#toast.error:not([hidden])")`, "wrong password rejected", 15000);
  await evaluate(`document.querySelector("#dialog-form").password.value = "correct horse";`);
  await evaluate(click(`#dialog button[type=submit]`));
  await until(`!!document.querySelector("[data-action=vault-password]")`, "unlocked", 15000);
  await evaluate(click(`[data-action=vault-password]`));
  await until(`document.querySelector("#dialog").open`, "change password dialog");
  await evaluate(`const f = document.querySelector("#dialog-form"); f.current.value = "correct horse"; f.new.value = "battery staple"; f.confirm.value = "battery staple";`);
  await evaluate(click(`#dialog button[type=submit]`));
  await until(`${toastText}.includes("changed")`, "password changed", 20000);
  await until(`!!document.querySelector("[data-action=vault-reset]")`, "reset button");
  await evaluate(click(`[data-action=vault-reset]`));
  await until(`document.querySelector("#dialog").open`, "reset confirm");
  await evaluate(click(`#dialog button[type=submit]`));
  await until(`!!document.querySelector("[data-action=vault-create]")`, "vault reset", 10000);
  await evaluate(click(`[data-action=delete-s3]`));
  await until(`document.querySelector("#dialog").open`, "delete confirm");
  await evaluate(click(`#dialog button[type=submit]`));
  await until(`!document.querySelector("[data-action=delete-s3]")`, "config deleted");
});

await step("Settings page", async () => {
  await evaluate(click(`[data-action=page][data-page=settings]`));
  await until(`!!document.querySelector("#settings-form")`, "settings form");
  await shot("app-settings-1440-dark");
  check(await evaluate(`return [...document.querySelectorAll("#settings-form .field label")].every((l) => document.getElementById(l.htmlFor))`), "settings fields are labelled");
  check(await evaluate(`return document.querySelector("[name=web_password]").value === ""`), "password not prefilled");
  await evaluate(`const f = document.querySelector("#settings-form"); f.public_url.value = "ftp://scratchpad.example.com"; f.requestSubmit();`);
  await until(`document.querySelector("#toast.error:not([hidden])")?.textContent.includes("Public URL")`, "invalid public URL rejected");
  await evaluate(`const f = document.querySelector("#settings-form"); f.public_url.value = ""; f.retention_days.value = "30"; f.requestSubmit();`);
  await until(`${toastText}.includes("Settings saved")`, "settings saved");
  check((await api("/settings")).retention_days === 30, "retention stored");
});

await step("Remote mailboxes page", async () => {
  await evaluate(click(`[data-action=page][data-page=remotes]`));
  await until(`!!document.querySelector("[data-action=poll-remotes]")`, "remotes page");
  await evaluate(click(`[data-action=edit-remote][data-id=""]`));
  await until(`document.querySelector("#dialog").open`, "remote dialog");
  await evaluate(`const f = document.querySelector("#dialog-form"); f.name.value = "Staging"; f.type.value = "scratchpad"; f.type.dispatchEvent(new Event("change", { bubbles: true })); f.enabled.checked = false;`);
  check(await evaluate(`return document.querySelector(".imap-only").hidden && !document.querySelector(".scratchpad-only").hidden`), "source fields switch");
  await evaluate(`const f = document.querySelector("#dialog-form"); f.base_url.value = "not a url"; f.requestSubmit();`);
  await until(`document.querySelector("#toast.error:not([hidden])")`, "invalid URL rejected");
  await evaluate(`const f = document.querySelector("#dialog-form"); f.base_url.value = "http://127.0.0.1:9"; f.h_password.value = "pw"; f.h_password.dispatchEvent(new Event("input", { bubbles: true })); f.requestSubmit();`);
  await until(`${count(".page .card")} === 1`, "remote saved");
  await shot("app-remotes-1440-dark");
  await evaluate(click(`[data-action=delete-remote]`));
  await until(`document.querySelector("#dialog").open`, "delete confirm");
  await evaluate(click(`#dialog button[type=submit]`));
  await until(`${count(".page .card")} === 0`, "remote deleted");
});

await step("Setup page, test email and live updates", async () => {
  await evaluate(click(`[data-action=page][data-page=setup]`));
  await until(`!!document.querySelector("[data-action=test-email]")`, "setup page");
  await shot("app-setup-1440-dark");
  await evaluate(click(`[data-action=test-email]`));
  await until(`${toastText}.includes("Test email sent")`, "test email sent", 10000);
  await until(`[...document.querySelectorAll(".side-item .name")].some((n) => n.textContent === "Test")`, "new mailbox arrives over SSE", 10000);
  await evaluate(click(`.side-item[data-name="__all__"]`));
  await until(`${count(".email-row")} === 6`, "list includes the test email");
  // A delivery while the list is open shows up without a reload.
  await sendMail("Smoke", PLAIN(9));
  await until(`${count(".email-row")} === 7`, "SSE adds new mail live", 10000);
});

await step("Delete with keyboard and empty states", async () => {
  await evaluate(click(".email-row"));
  await until(`!!document.querySelector(".detail-head")`, "selected");
  await evaluate(`document.activeElement?.blur()`);
  await key("Backspace");
  await until(`${count(".email-row")} === 6`, "Backspace deletes");
  await evaluate(click(`.side-item[data-name="Laravel"]`));
  await until(`!!document.querySelector("[data-action=delete-mailbox]")`, "Laravel mailbox");
  await evaluate(click(`[data-action=delete-mailbox]`));
  await until(`document.querySelector("#dialog").open`, "confirm");
  await evaluate(click(`#dialog button[type=submit]`));
  await until(`!document.querySelector('.side-item[data-name="Laravel"]')`, "mailbox deleted");
});

await step("Themes and narrow layouts", async () => {
  await evaluate(click(`.side-item[data-name="__all__"]`));
  await until(`${count(".email-row")} >= 5`, "all emails");
  await evaluate(`[...document.querySelectorAll(".email-row")].find((r) => r.textContent.includes("Ada Lovelace")).click()`);
  await until(`!!document.querySelector(".dv-stage iframe")`, "preview");
  await evaluate(click(`[data-theme="light"]`));
  check(await evaluate(`return document.documentElement.dataset.theme === "light" && localStorage.getItem("scratchpad-theme") === "light"`), "light theme");
  await shot("app-1440-light");
  await viewport(390, 844, false);
  await sleep(300);
  check(await evaluate(`return document.documentElement.scrollWidth <= 391`), "no horizontal overflow at 390 px (light)");
  await shot("app-390-light");
  await evaluate(click(`[data-theme="dark"]`));
  await shot("app-390-dark");
  await evaluate(click(`[data-action=page][data-page=settings]`));
  await until(`!!document.querySelector("#settings-form")`, "settings on phone");
  check(await evaluate(`return document.documentElement.scrollWidth <= 391`), "settings fit at 390 px");
  await shot("app-settings-390-dark");
  await evaluate(click(`[data-action=page][data-page=setup]`));
  await shot("app-setup-390-dark");
  await evaluate(click(`[data-theme="system"]`));
  await viewport(1440, 900, true);
});

// ------------------------------------------------------------------ export viewer

await step("Exported file from file://", async () => {
  const ids = (await api("/emails")).map((e) => e.id);
  const response = await fetch(`${BASE}/api/export`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ ids, title: "Smoke export" }) });
  check(response.ok, "export endpoint");
  const file = join(temp, "export.html");
  writeFileSync(file, Buffer.from(await response.arrayBuffer()));
  requests.length = 0;
  await navigate(pathToFileURL(file).href);
  await until(`${count(".x-app .email-row")} === ${ids.length}`, "all emails in the viewer");
  for (const index of [...ids.keys()]) {
    await evaluate(click(`.email-row[data-index="${index}"]`));
    await sleep(100);
    const tabs = await evaluate(`return [...document.querySelectorAll(".tabs .tab")].map((t) => t.dataset.tab)`);
    for (const tab of tabs) {
      await evaluate(click(`.tab[data-tab="${tab}"]`));
      await sleep(60);
    }
  }
  await evaluate(`[...document.querySelectorAll(".email-row")].find((r) => r.textContent.includes("Ada Lovelace")).click()`);
  await evaluate(click(`.tab[data-tab="preview"]`));
  await until(`document.querySelector(".dv-stage iframe")?.contentDocument?.querySelector("img")?.naturalWidth === 1`, "embedded cid image renders");
  await evaluate(click(`.dv-switch [data-device="all"]`));
  await shot("export-1440-dark");
  await evaluate(click(`.tab[data-tab="links"]`));
  const hrefs = await evaluate(`return [...document.querySelectorAll("#tab-body a")].map((a) => a.getAttribute("href"))`);
  check(hrefs.every((h) => /^https?:/.test(h)), "export: only http(s) links are clickable");
  await evaluate(click(`.tab[data-tab="attachments"]`));
  await evaluate(click(`[data-download="2"]`));
  await evaluate(click(`[data-theme-pref="light"]`));
  check(await evaluate(`return document.documentElement.dataset.theme === "light"`), "export theme toggle");
  await viewport(390, 844, false);
  await evaluate(click(`.tab[data-tab="preview"]`));
  await sleep(200);
  check(await evaluate(`return document.documentElement.scrollWidth <= 391`), "export fits at 390 px");
  await shot("export-390-light");
  await viewport(1440, 900, true);
  check(await evaluate(`return window.__xss === undefined && document.querySelector(".dv-stage iframe").getAttribute("sandbox").includes("allow-scripts") === false`), "export: email scripts don't run");
  const remote = requests.filter((url) => !/^(file|data|blob|about):/.test(url));
  check(!remote.length, `export makes no network requests (${remote.join(", ") || "none"})`);
});

ws.close();
console.log(failures ? `\n${failures} check(s) failed` : "\nAll UI checks passed");
if (SHOTS) console.log(`Screenshots in ${SHOTS}`);
process.exit(failures ? 1 : 0);
