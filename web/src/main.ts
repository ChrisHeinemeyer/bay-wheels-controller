import { initFlashTab }  from "./flash";
import { initWifiTab }   from "./wifi";
import { initStatusTab } from "./status";

// ── Safari check ──────────────────────────────────────────────────────────────
const isSafari = !!(window as Window & { safari?: unknown }).safari;
if (isSafari) {
  (document.getElementById("safariErr") as HTMLElement).style.display = "block";
  (document.getElementById("main")      as HTMLElement).style.display = "none";
}

// ── Tab elements ──────────────────────────────────────────────────────────────
const flashTab   = document.getElementById("flashTab")!;
const wifiTab    = document.getElementById("wifiTab")!;
const statusTab  = document.getElementById("statusTab")!;
const tabButtons = document.querySelectorAll<HTMLButtonElement>(".tab");

// ── Tab switching ─────────────────────────────────────────────────────────────
// `onWifiActivate` is filled in below after initWifiTab() runs.
let onWifiActivate: () => void = () => {};

function showTab(name: "flash" | "wifi" | "status") {
  flashTab.style.display  = name === "flash"  ? "" : "none";
  wifiTab.style.display   = name === "wifi"   ? "" : "none";
  statusTab.style.display = name === "status" ? "" : "none";
  tabButtons.forEach(btn => btn.classList.toggle("active", btn.dataset.tab === name));
  if (name === "wifi") onWifiActivate();
}

tabButtons.forEach(btn => {
  btn.addEventListener("click", () => showTab(btn.dataset.tab as "flash" | "wifi" | "status"));
});

// ── Init tabs ─────────────────────────────────────────────────────────────────
// `justFlashed` is owned here so both flash and wifi tabs can read/write it
// without coupling to each other directly.
let justFlashed = false;

onWifiActivate = initWifiTab(() => justFlashed);

initFlashTab(() => {
  justFlashed = true;
  showTab("wifi");
});

initStatusTab();
