// Nav pill drag behavior (owner request, 2026-11-04): the injected
// back/forward/reload pill can be dragged anywhere on screen, the
// position is remembered per origin, a drag never fires the button
// it started on, and a double-click on the grip restores the
// default corner. Runs against the real BRIDGE_JS extracted from
// lib.rs, inside jsdom (no layout: rects are 0, so assertions use
// the values the drag math writes to style/localStorage).
import { JSDOM } from "jsdom";
import { extractBridge } from "./extract-bridge.mjs";
const bridge = extractBridge();
let pass = 0, fail = 0;
const ok = (name, cond) => { cond ? pass++ : fail++; console.log((cond ? "PASS " : "FAIL ") + name); };
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
function makePage(savedPos) {
  const dom = new JSDOM(`<!DOCTYPE html><html><body><div class="o_action_manager"><div>x</div></div></body></html>`, {
    url: "http://68.221.27.248:8070/odoo", runScripts: "dangerously", pretendToBeVisual: true,
  });
  const w = dom.window;
  w.__TAURI_INTERNALS__ = { invoke: async () => ({}) };
  if (savedPos) w.localStorage.setItem("npNavPillPos", JSON.stringify(savedPos));
  const s = w.document.createElement("script"); s.textContent = bridge; w.document.body.appendChild(s);
  return { dom, w };
}
function pointer(w, type, x, y) {
  return new w.MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 0 });
}

// A: default pill — 3 buttons + grip, no custom position.
{
  const { w } = makePage(null);
  await sleep(30);
  const pill = w.document.getElementById("np-nav-pill");
  ok("pill: exists on backend page", !!pill);
  ok("pill: exactly 3 buttons", pill && pill.querySelectorAll("button").length === 3);
  ok("pill: has a drag grip", pill && !!pill.querySelector(".np-pill-grip"));
  ok("pill: default corner (no inline left/top)", pill && pill.style.left === "" && pill.style.top === "");
  w.close();
}
// B: drag moves the pill and remembers the drop point.
{
  const { w } = makePage(null);
  await sleep(30);
  const pill = w.document.getElementById("np-nav-pill");
  pill.dispatchEvent(pointer(w, "pointerdown", 100, 60));
  pill.dispatchEvent(pointer(w, "pointermove", 170, 120));
  ok("drag: pill follows the pointer", pill.style.left === "70px" && pill.style.top === "60px");
  ok("drag: dragging class applied", pill.classList.contains("np-dragging"));
  pill.dispatchEvent(pointer(w, "pointerup", 170, 120));
  ok("drag: dragging class removed on drop", !pill.classList.contains("np-dragging"));
  const saved = JSON.parse(w.localStorage.getItem("npNavPillPos") || "null");
  ok("drag: position persisted to localStorage", !!saved && isFinite(saved.x) && isFinite(saved.y));
  w.close();
}
// C: the click that ends a drag is swallowed; a plain click is not.
{
  const { w } = makePage(null);
  await sleep(30);
  const pill = w.document.getElementById("np-nav-pill");
  const firstBtn = pill.querySelector("button");
  pill.dispatchEvent(pointer(w, "pointerdown", 100, 60));
  pill.dispatchEvent(pointer(w, "pointermove", 200, 160));
  pill.dispatchEvent(pointer(w, "pointerup", 200, 160));
  const afterDrag = firstBtn.dispatchEvent(new w.MouseEvent("click", { bubbles: true, cancelable: true }));
  ok("drag: trailing click swallowed", afterDrag === false);
  await sleep(120); // let the suppression window lapse
  const plain = firstBtn.dispatchEvent(new w.MouseEvent("click", { bubbles: true, cancelable: true }));
  ok("click: plain button click not swallowed", plain === true);
  w.close();
}
// D: a saved position is restored (and clamped) on the next load.
{
  const { w } = makePage({ x: 300, y: 200 });
  await sleep(30);
  const pill = w.document.getElementById("np-nav-pill");
  ok("restore: saved position applied", pill && pill.style.left === "300px" && pill.style.top === "200px");
  w.close();
}
{
  const { w } = makePage({ x: 99999, y: 200 });
  await sleep(30);
  const pill = w.document.getElementById("np-nav-pill");
  ok("restore: off-screen position clamped into the viewport",
    pill && pill.style.left !== "99999px" && parseInt(pill.style.left, 10) < 1024);
  w.close();
}
// E: double-click on the grip resets to the default corner.
{
  const { w } = makePage({ x: 300, y: 200 });
  await sleep(30);
  const pill = w.document.getElementById("np-nav-pill");
  const grip = pill.querySelector(".np-pill-grip");
  grip.dispatchEvent(new w.MouseEvent("dblclick", { bubbles: true, cancelable: true }));
  ok("reset: inline position cleared", pill.style.left === "" && pill.style.top === "");
  ok("reset: saved position removed", w.localStorage.getItem("npNavPillPos") === null);
  w.close();
}
console.log(`\nNAVPILL: ${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
