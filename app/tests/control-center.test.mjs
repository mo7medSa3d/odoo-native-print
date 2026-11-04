import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const html = readFileSync(new URL("../dist/index.html", import.meta.url), "utf8");
const scripts = [...html.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/gi)];
assert.equal(scripts.length, 1, "Control Center must have one embedded app script");
const script = scripts[0][1];

test("embedded Control Center JavaScript parses", () => {
  assert.doesNotThrow(() => new vm.Script(script));
});

test("inline button and field handlers resolve to functions", () => {
  const functions = new Set([...script.matchAll(/(?:async\s+)?function\s+([\w$]+)\s*\(/g)].map(m => m[1]));
  const handlers = [...html.matchAll(/\bon(?:click|change|input|submit)=["']([^"']+)["']/gi)];
  assert.ok(handlers.length >= 20);
  for (const handler of handlers) {
    const name = handler[1].match(/^\s*([\w$]+)\s*\(/)?.[1];
    assert.ok(name && functions.has(name), "unresolved handler: " + handler[1]);
  }
});

test("static DOM lookups resolve and element IDs are unique", () => {
  // (?<![-\w]) keeps data-* attributes (e.g. data-job-id) out: only
  // real element id attributes count for uniqueness.
  const ids = [...html.matchAll(/(?<![-\w])id=["']([^"']+)["']/g)].map(m => m[1]);
  const set = new Set(ids);
  assert.equal(set.size, ids.length);
  const refs = [...script.matchAll(/getElementById\(["']([^"'$]+)["']\)/g)].map(m => m[1]);
  assert.deepEqual([...new Set(refs.filter(id => !set.has(id)))], []);
});

test("localized controls have Arabic and English translations", () => {
  const match = script.match(/const I18N = (\{[\s\S]*?\n  \});/);
  assert.ok(match);
  const i18n = new vm.Script("(" + match[1] + ")").runInNewContext({});
  for (const key of html.matchAll(/data-i18n=["']([^"']+)["']/g)) {
    assert.ok(i18n.ar[key[1]], "missing Arabic translation: " + key[1]);
    assert.ok(i18n.en[key[1]], "missing English translation: " + key[1]);
  }
});

function extract(name, marker) {
  const start = script.indexOf("function " + name + "(");
  const end = script.indexOf(marker, start);
  assert.ok(start >= 0 && end > start, "function not found: " + name);
  return script.slice(start, end).trim();
}
const validOdoo = new vm.Script("(" + extract("isAllowedOrigin", "  function isAllowedPrinterUri(") + ")").runInNewContext({ URL });
const validPrinter = new vm.Script("(" + extract("isAllowedPrinterUri", "  // Live field validation:") + ")").runInNewContext({ URL });

test("Odoo origin accepts HTTPS anywhere and HTTP on any IP", () => {
  for (const value of [
    "https://odoo.company.local:8069", "https://odoo.example.com",
    "https://192.168.1.20:8069", "https://8.8.8.8:8069",
    "http://192.168.1.20:8069", "http://10.0.0.5:8069",
    "http://172.16.4.9:8069", "http://127.0.0.1:8069",
    "http://8.8.8.8:8069", "http://68.221.27.248:8070",
    "http://[::1]:8069", "http://[fd00::25]:8069", "http://[fe80::1]:8069",
    "http://[2001:4860:4860::8888]:8069"
  ]) assert.equal(validOdoo(value), true, value);
});

test("Odoo origin rejects HTTP hostnames, bad IPs and URL tricks", () => {
  for (const value of [
    "http://odoo.company.local:8069",
    "http://999.999.999.999:8069",
    "http://192.168.1.20:0", "http://192.168.1.20:65536",
    "http://user@192.168.1.20:8069", "https://user@odoo.example.com:8069",
    "http://192.168.1.20:8069/web", "https://odoo.example.com/web",
    "http://192.168.1.20:8069/?debug=1", "http://192.168.1.20:8069#x",
    "ftp://192.168.1.20:8069"
  ]) assert.equal(validOdoo(value), false, value);
});

test("printer URI accepts IP-only TCP, RAW and IPP", () => {
  for (const value of [
    "tcp://192.168.1.200:9100", "raw://192.168.1.200:9100",
    "ipp://192.168.1.200:631/ipp/print", "ipp://[::1]:631/ipp/print"
  ]) assert.equal(validPrinter(value), true, value);
});

test("printer URI rejects domains, other schemes and bad ports", () => {
  for (const value of [
    "tcp://printer.local:9100", "https://192.168.1.200:9100",
    "ftp://192.168.1.200:21", "tcp://999.999.999.999:9100",
    "tcp://192.168.1.200:0", "tcp://192.168.1.200:65536",
    "tcp://192.168.1.200:9100/print"
  ]) assert.equal(validPrinter(value), false, value);
});

test("demo fallback and local Windows paths are removed", () => {
  assert.equal(html.includes("FALLBACK_DEMO"), false);
  assert.equal(html.includes("Preview:"), false);
  assert.equal(html.includes("txtConfigPath"), false);
  assert.equal(html.includes("txtPdfiumPath"), false);
  assert.match(script, /async function saveAndOpenOdoo\(\)/);
  assert.match(script, /document\.execCommand\("copy"\)/);
  assert.match(script, /window\.setTimeout\(\(\) =>/);
});
