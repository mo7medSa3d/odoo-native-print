// Extract the bridge script exactly as the app injects it: the
// BRIDGE_JS raw string from app/src-tauri/src/lib.rs, with the
// __APP_VERSION__ placeholder substituted the same way lib.rs does
// at compile time (env!("CARGO_PKG_VERSION")).
import fs from "node:fs";

export function extractBridge() {
  const lib = fs.readFileSync(
    new URL("../../src-tauri/src/lib.rs", import.meta.url), "utf8");
  const start = lib.indexOf('const BRIDGE_JS: &str = r#"') + 'const BRIDGE_JS: &str = r#"'.length;
  const end = lib.indexOf('"#;', start);
  if (start < 0 || end < 0) throw new Error("BRIDGE_JS raw string not found in lib.rs");
  const conf = JSON.parse(fs.readFileSync(
    new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
  return lib.slice(start, end).replaceAll("__APP_VERSION__", conf.version);
}
