import { ESPLoader, Transport, FlashOptions } from "esptool-js";

// Resolves relative to the current page, so it works in both local dev
// (Vite proxy intercepts /firmware-bay-wheels-controller.bin) and on GitHub Pages
// (bundled file is served from /bay-wheels-controller/firmware-bay-wheels-controller.bin).
// will this work?
const FIRMWARE_URL = new URL("firmware-bay-wheels-controller.bin", window.location.href).href;

const CHUNK = 8192;
function uint8ArrayToBinaryString(arr: Uint8Array): string {
  let s = "";
  for (let i = 0; i < arr.length; i += CHUNK)
    s += String.fromCharCode(...arr.subarray(i, i + CHUNK));
  return s;
}

// ── Safari check ──────────────────────────────────────────────────────────────
const isSafari = !!(window as Window & { safari?: unknown }).safari;
if (isSafari) {
  (document.getElementById("safariErr") as HTMLElement).style.display = "block";
  (document.getElementById("main") as HTMLElement).style.display = "none";
}

// ── DOM refs ──────────────────────────────────────────────────────────────────
const flashTab         = document.getElementById("flashTab")!;
const wifiTab          = document.getElementById("wifiTab")!;
const statusTab        = document.getElementById("statusTab")!;
const tabButtons       = document.querySelectorAll<HTMLButtonElement>(".tab");

const connectButton    = document.getElementById("connectButton") as HTMLButtonElement;
const disconnectButton = document.getElementById("disconnectButton") as HTMLButtonElement;
const programButton    = document.getElementById("programButton") as HTMLButtonElement;
const flashOptions     = document.getElementById("flashOptions")!;
const lblConnTo        = document.getElementById("lblConnTo")!;
const terminal         = document.getElementById("terminal")!;
const firmwareFile     = document.getElementById("firmwareFile") as HTMLInputElement;

const wifiInstructions     = document.getElementById("wifiInstructions")!;
const wifiInstructionsText = document.getElementById("wifiInstructionsText")!;
const wifiConnectBtn       = document.getElementById("wifiConnectBtn") as HTMLButtonElement;
const ssidInput            = document.getElementById("ssidInput") as HTMLInputElement;
const passwordInput        = document.getElementById("passwordInput") as HTMLInputElement;
const wifiConfigBtn        = document.getElementById("wifiConfigBtn") as HTMLButtonElement;
const wifiStatus           = document.getElementById("wifiStatus")!;
const wifiSpinner          = document.getElementById("wifiSpinner")!;
const wifiStatusIcon       = document.getElementById("wifiStatusIcon")!;
const wifiStatusText       = document.getElementById("wifiStatusText")!;

// ── State ─────────────────────────────────────────────────────────────────────
let device: SerialPort | null = null;
let transport: Transport | null = null;
let esploader: ESPLoader | null = null;
// Set to true after a successful flash so WiFi tab knows the device is ready
let justFlashed = false;
// Active serial reader — must be cancelled before closing the port
let activeReader: ReadableStreamDefaultReader<Uint8Array> | null = null;

// ── Port teardown ─────────────────────────────────────────────────────────────
async function releasePort() {
  if (activeReader) {
    try { await activeReader.cancel(); } catch { /* ignore */ }
    try { activeReader.releaseLock(); } catch { /* ignore */ }
    activeReader = null;
  }
  if (device) {
    try { await device.close(); } catch { /* ignore */ }
    device = null;
  }
}

// ── Tab switching ─────────────────────────────────────────────────────────────
function showTab(name: "flash" | "wifi" | "status") {
  flashTab.style.display  = name === "flash"  ? "" : "none";
  wifiTab.style.display   = name === "wifi"   ? "" : "none";
  statusTab.style.display = name === "status" ? "" : "none";
  tabButtons.forEach(btn => btn.classList.toggle("active", btn.dataset.tab === name));
  if (name === "wifi") setupWifiTab();
}

tabButtons.forEach(btn => {
  btn.addEventListener("click", () => showTab(btn.dataset.tab as "flash" | "wifi" | "status"));
});

// Submit WiFi credentials on Enter from either input field
[ssidInput, passwordInput].forEach(input => {
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") wifiConfigBtn.click();
  });
});

// ── Flash console ─────────────────────────────────────────────────────────────
function log(msg: string) {
  terminal.textContent += msg + "\n";
  terminal.scrollTop = terminal.scrollHeight;
}
function logClear() { terminal.textContent = ""; }

const espLoaderTerminal = {
  clean: () => logClear(),
  writeLine: (data: string) => log(data),
  write: (data: string) => { terminal.textContent += data; terminal.scrollTop = terminal.scrollHeight; },
};

// ── Flash: connect ────────────────────────────────────────────────────────────
connectButton.onclick = async () => {
  if (!("serial" in navigator)) {
    alert("Web Serial API is not supported. Use Chrome or Edge.");
    return;
  }
  try {
    // Cancel any in-flight provisioning reader and close any open port before
    // handing a fresh handle to esptool.
    await releasePort();
    device = await navigator.serial.requestPort();
    transport = new Transport(device);
    esploader = new ESPLoader({
      transport,
      baudrate: 921600,
      romBaudrate: 115200,
      terminal: espLoaderTerminal,
    });
    const chip = await esploader.main();

    lblConnTo.textContent = "Connected: " + chip;
    lblConnTo.style.display = "inline";
    connectButton.style.display = "none";
    disconnectButton.style.display = "inline";
    flashOptions.style.display = "block";
  } catch (e) {
    log("Error: " + (e instanceof Error ? e.message : String(e)));
  }
};

// ── Flash: disconnect ─────────────────────────────────────────────────────────
disconnectButton.onclick = async () => {
  if (transport) await transport.disconnect();
  logClear();
  connectButton.style.display = "inline";
  disconnectButton.style.display = "none";
  lblConnTo.style.display = "none";
  flashOptions.style.display = "none";
  device = null; transport = null; esploader = null; justFlashed = false;
};

// ── Flash: program ────────────────────────────────────────────────────────────
programButton.onclick = async () => {
  if (!esploader) return;

  const source = (document.querySelector('input[name="firmware"]:checked') as HTMLInputElement)?.value;
  let firmwareData: Uint8Array;

  if (source === "file" && firmwareFile.files?.length) {
    firmwareData = new Uint8Array(await firmwareFile.files[0].arrayBuffer());
  } else {
    log("Downloading firmware from latest release...");
    const res = await fetch(FIRMWARE_URL);
    if (!res.ok) throw new Error("Failed to download firmware: " + res.status);
    firmwareData = new Uint8Array(await res.arrayBuffer());
    log(`Downloaded ${(firmwareData.length / 1024).toFixed(1)} KB`);
  }

  const progressEl = document.getElementById("progress0")!;

  try {
    programButton.disabled = true;
    const options: FlashOptions = {
      fileArray: [{ address: 0x0, data: uint8ArrayToBinaryString(firmwareData) }],
      flashSize: "keep",
      flashMode: "keep",
      flashFreq: "keep",
      eraseAll: true,
      compress: true,
      reportProgress: (_i, written, total) => {
        progressEl.textContent = total > 0 ? Math.round((written / total) * 100) + "%" : "";
      },
    };
    await esploader.writeFlash(options);
    await esploader.after("hard_reset", true);
    log("Firmware flashed! Switching to WiFi setup...");

    // The hard reset causes USB re-enumeration — the old SerialPort handle is
    // invalid after the device reappears. Drop it entirely and let the user
    // connect fresh in the WiFi tab (same path that already works reliably).
    try { await transport!.disconnect(); } catch { /* ignore */ }
    transport = null; esploader = null;
    try { await device!.close(); } catch { /* ignore */ }
    device = null;

    justFlashed = true;
    showTab("wifi");
  } catch (e) {
    log("Error: " + (e instanceof Error ? e.message : String(e)));
  } finally {
    programButton.disabled = false;
    progressEl.textContent = "";
  }
};

// ── WiFi tab setup ────────────────────────────────────────────────────────────
function setupWifiTab() {
  setWifiStatus("idle");

  if (!device) {
    if (justFlashed) {
      // Post-flash: USB re-enumerates after hard reset, needs a fresh connection
      wifiInstructionsText.textContent =
        "Firmware flashed! Unplug the device, plug it back in, then click Connect.";
    } else {
      // Standalone: user needs to enter provisioning mode via GPIO2
      wifiInstructionsText.innerHTML =
        "Hold the <strong>GPIO2</strong> button while power cycling your board " +
        "to enter WiFi setup mode, then click Connect.";
    }
    wifiInstructions.style.display = "block";
    wifiConnectBtn.style.display = "inline-block";
  } else {
    wifiInstructions.style.display = "none";
  }
}

wifiConnectBtn.onclick = async () => {
  if (!("serial" in navigator)) {
    alert("Web Serial API is not supported. Use Chrome or Edge.");
    return;
  }
  try {
    device = await navigator.serial.requestPort();
    wifiInstructions.style.display = "none";
  } catch (e) {
    if ((e as Error).name !== "NotAllowedError")
      alert("Could not open port: " + (e instanceof Error ? e.message : String(e)));
  }
};

// ── WiFi status helpers ───────────────────────────────────────────────────────
type WifiStatusState = "idle" | "working" | "success" | "error";

function setWifiStatus(state: WifiStatusState, message = "") {
  wifiStatus.style.display = state === "idle" ? "none" : "flex";
  wifiSpinner.style.display = state === "working" ? "inline-block" : "none";
  wifiStatusIcon.style.display = state === "success" || state === "error" ? "inline" : "none";
  wifiStatusText.textContent = message;

  wifiConfigBtn.disabled = state === "working";
  ssidInput.disabled = state === "working";
  passwordInput.disabled = state === "working";

  if (state === "success") {
    wifiStatusIcon.textContent = "✓";
    wifiStatusIcon.className = "status-icon success";
  } else if (state === "error") {
    wifiStatusIcon.textContent = "✗";
    wifiStatusIcon.className = "status-icon error";
  } else {
    wifiStatusIcon.className = "status-icon";
  }
}

// ── WiFi: configure button ────────────────────────────────────────────────────
wifiConfigBtn.onclick = async () => {
  const ssid = ssidInput.value.trim();
  if (!ssid) { ssidInput.focus(); return; }
  if (!device) {
    alert("Connect to your device first.");
    return;
  }

  setWifiStatus("working", "Connecting...");
  try {
    if (!device.readable) {
      await device.open({ baudRate: 115200 });
    }
    await runProvisioning(
      device,
      ssid,
      passwordInput.value,
      (msg, done, isError) => {
        if (done)        setWifiStatus("success", msg);
        else if (isError) setWifiStatus("error", msg);
        else              setWifiStatus("working", msg);
      }
    );
  } catch (e) {
    setWifiStatus("error", e instanceof Error ? e.message : String(e));
  } finally {
    await releasePort();
  }
};

// ── Provisioning state machine ────────────────────────────────────────────────
function provLog(msg: string) {
  console.log(`[provisioning] ${msg}`);
}

async function runProvisioning(
  port: SerialPort,
  ssid: string,
  password: string,
  onStatus: (msg: string, done?: boolean, error?: boolean) => void
): Promise<void> {
  const dec = new TextDecoder();
  const enc = new TextEncoder();
  let buf = "";
  type State = "ssid" | "pwd" | "confirm" | "result";
  let state: State = "ssid";

  async function send(text: string) {
    provLog(`→ sending: ${text === password ? "***" : JSON.stringify(text)}`);
    const writer = port.writable!.getWriter();
    // Firmware's drain_newline() reads one extra byte after \r or \n, expecting CRLF.
    // Send \r\n so drain_newline consumes the \n and doesn't block.
    try { await writer.write(enc.encode(text + "\r\n")); }
    finally { writer.releaseLock(); }
  }

  async function ensureOpen() {
    provLog(`port.readable=${port.readable != null}`);
    if (!port.readable) {
      provLog("opening port at 115200...");
      await port.open({ baudRate: 115200 });
      provLog("port opened");
    }
  }

  const deadline = Date.now() + 120_000; // 2 min overall timeout
  provLog("starting (state=ssid)");
  onStatus("Waiting for device...");

  await ensureOpen();
  let reader = port.readable!.getReader();
  activeReader = reader;
  provLog("reader acquired, entering read loop");

  try {
    while (Date.now() < deadline) {
      let value: Uint8Array | undefined;

      try {
        // Race the read against a 5 s timeout so we don't block forever
        const raceResult = await Promise.race([
          reader.read() as Promise<ReadableStreamReadResult<Uint8Array>>,
          new Promise<ReadableStreamReadResult<Uint8Array>>(resolve =>
            setTimeout(() => resolve({ value: new Uint8Array(0), done: false }), 5_000)
          ),
        ]);
        if (raceResult.done) {
          provLog("stream done — USB re-enumerated");
          throw new Error("stream-ended");
        }
        value = raceResult.value;
        if (value && value.length > 0) {
          const chunk = dec.decode(value);
          provLog(`← rx (${value.length}B): ${JSON.stringify(chunk)}`);
        } else {
          provLog(`← timeout tick (state=${state}, buf=${JSON.stringify(buf.slice(-80))})`);
        }
      } catch (err) {
        provLog(`read error: ${err} — assuming mid-reboot disconnect`);
        // Port disconnected mid-reboot; release lock, wait, then reopen
        try { reader.releaseLock(); } catch { /* empty */ }
        onStatus("Device rebooting, waiting...");
        try { await port.close(); } catch { /* empty */ }
        provLog("sleeping 3 s before reopen...");
        await sleep(3_000);
        provLog("attempting reopen...");
        await ensureOpen().catch((e) => provLog(`reopen failed: ${e}`));
        reader = port.readable!.getReader();
        activeReader = reader;
        provLog("reader re-acquired after reboot");
        buf = "";
        continue;
      }

      if (value && value.length > 0) buf += dec.decode(value);

      if (state === "ssid") {
        if (buf.includes("Enter WiFi SSID:")) {
          provLog("matched 'Enter WiFi SSID:' → sending SSID, moving to pwd");
          onStatus("Sending SSID...");
          await send(ssid);
          state = "pwd"; buf = "";
        }
      } else if (state === "pwd") {
        if (buf.includes("Enter WiFi SSID:")) {
          // Speculative send landed before the device was ready — re-send SSID now
          provLog("saw SSID prompt while in pwd state — re-sending SSID");
          onStatus("Sending SSID...");
          await send(ssid);
          buf = "";
        } else if (buf.includes("Enter WiFi Password:")) {
          provLog("matched 'Enter WiFi Password:' → sending password, moving to confirm");
          onStatus("Sending password...");
          await send(password);
          state = "confirm"; buf = "";
        }
      } else if (state === "confirm") {
        if (buf.includes("Save credentials?")) {
          provLog("matched 'Save credentials?' → sending y, moving to result");
          onStatus("Saving...");
          await send("y");
          state = "result"; buf = "";
        }
      } else if (state === "result") {
        if (buf.includes("Credentials saved successfully!")) {
          provLog("matched 'Credentials saved successfully!' → done!");
          onStatus("WiFi configured!", true);
          return;
        }
        if (buf.includes("Error saving credentials")) {
          provLog("matched 'Error saving credentials' → waiting for reboot + re-prompt");
          // Device will reboot and re-enter provisioning — just wait for the next SSID prompt
          onStatus("Rebooting to retry, please wait...");
          state = "ssid"; buf = "";
        }
      }
    }
    provLog("timed out after 2 minutes");
    throw new Error("Configuration timed out after 2 minutes");
  } finally {
    try { reader.releaseLock(); } catch { /* empty */ }
    activeReader = null;
    provLog("reader released");
  }
}

function sleep(ms: number) { return new Promise(r => setTimeout(r, ms)); }

// ── Status tab ────────────────────────────────────────────────────────────────

// Frame layout: 46 bytes
// [0]    magic   = 0xAB
// [1]    battery_pct  u8
// [2]    wifi_connected  u8 (0/1)
// [3]    rssi  i8 (as u8 bits)
// [4..8] fetch_age_secs  u32 LE  (0xFFFFFFFF = never)
// [8]    station_input  u8
// [9..45] led_rgb  [u8; 36]  (12 LEDs × r,g,b)
// [45]   XOR checksum of bytes 0–44
const STATUS_MAGIC      = 0xAB;
const STATUS_FRAME_SIZE = 46;

const STATION_NAMES: Record<number, string> = {
  0:   "McAllister & Arguello",
  1:   "Arguello & Edward",
  2:   "Harrison & 17th St",
  3:   "Conservatory of Flowers",
  4:   "Arguello & Geary",
  5:   "7th Ave & Cabrillo",
  6:   "8th Ave & JFK",
  7:   "Turk & Stanyan",
  8:   "Parker & McAllister",
  9:   "Fell & Stanyan",
  10:  "Waller & Shrader",
  11:  "Page & Masonic",
  12:  "MLK & 7th Ave",
  13:  "Frederick & Arguello",
  14:  "5th Ave & Anza",
  15:  "7th Ave & Clement",
  255: "None",
};

interface LedColor { r: number; g: number; b: number; }
interface StatusFrame {
  batteryPct: number;
  wifiConnected: boolean;
  rssi: number;
  fetchAgeSecs: number;
  stationInput: number;
  leds: LedColor[];
}

function parseStatusFrame(buf: Uint8Array): StatusFrame {
  const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  return {
    batteryPct:    buf[1],
    wifiConnected: buf[2] === 1,
    rssi:          view.getInt8(3),
    fetchAgeSecs:  view.getUint32(4, true),
    stationInput:  buf[8],
    leds: Array.from({ length: 12 }, (_, i) => ({
      r: buf[9 + i * 3],
      g: buf[9 + i * 3 + 1],
      b: buf[9 + i * 3 + 2],
    })),
  };
}

// ── Status DOM refs ───────────────────────────────────────────────────────────
const statusConnectBtn    = document.getElementById("statusConnectBtn")    as HTMLButtonElement;
const statusDisconnectBtn = document.getElementById("statusDisconnectBtn") as HTMLButtonElement;
const statusConnLabel     = document.getElementById("statusConnLabel")!;
const statusDisplay       = document.getElementById("statusDisplay")!;
const statusBatteryVal    = document.getElementById("statusBatteryVal")!;
const statusBatteryBar    = document.getElementById("statusBatteryBar")    as HTMLElement;
const statusWifiDot       = document.getElementById("statusWifiDot")!;
const statusWifiText      = document.getElementById("statusWifiText")!;
const statusRssi          = document.getElementById("statusRssi")!;
const statusFetchAge      = document.getElementById("statusFetchAge")!;
const statusInput         = document.getElementById("statusInput")!;
const statusLedsEl        = document.getElementById("statusLeds")!;

// ── Status serial state ───────────────────────────────────────────────────────
let statusDevice: SerialPort | null = null;
let statusReader: ReadableStreamDefaultReader<Uint8Array> | null = null;

// ── Frame framing: accumulator-based with checksum verification ───────────────
//
// We accumulate raw bytes in a flat array, then scan for 0xAB. Once we have
// STATUS_FRAME_SIZE bytes starting with 0xAB, we verify the XOR checksum.
// On failure we discard one byte and re-scan — this correctly handles any data
// byte that happens to equal 0xAB (e.g. rssi == -85 dBm == 0xAB).
let frameAccum: number[] = [];

function resetFramer() { frameAccum = []; }

function frameChecksumValid(buf: number[]): boolean {
  const data = buf.slice(0, STATUS_FRAME_SIZE - 1);
  const checksum = data.reduce((acc, b) => acc ^ b, 0);
  return checksum === buf[STATUS_FRAME_SIZE - 1];
}

function processStatusBytes(bytes: Uint8Array) {
  for (const b of bytes) frameAccum.push(b);

  // Consume as many valid frames as possible from the front of the accumulator.
  while (frameAccum.length >= STATUS_FRAME_SIZE) {
    // Advance past any leading non-magic bytes.
    if (frameAccum[0] !== STATUS_MAGIC) { frameAccum.shift(); continue; }

    // We have a candidate frame. Verify checksum before accepting.
    if (frameChecksumValid(frameAccum)) {
      renderStatus(parseStatusFrame(new Uint8Array(frameAccum.slice(0, STATUS_FRAME_SIZE))));
      frameAccum.splice(0, STATUS_FRAME_SIZE);
    } else {
      // False magic byte — discard it and keep scanning.
      frameAccum.shift();
    }
  }
}

// ── LED grid initialisation (done once) ──────────────────────────────────────
// Physical LED layout matches the hardware:
//   Row 1 (top):    eBike  LEDs 0–5
//   Row 2 (bottom): Mech   LEDs 11–6  (reversed)
const LED_GRID_ORDER = [0, 1, 2, 3, 4, 5, 11, 10, 9, 8, 7, 6];

function initLedGrid() {
  statusLedsEl.innerHTML = "";
  for (const ledIdx of LED_GRID_ORDER) {
    const wrap = document.createElement("div");
    const circle = document.createElement("div");
    circle.className = "led-circle";
    circle.id = `led-${ledIdx}`;
    circle.style.backgroundColor = "rgb(20,20,20)";
    const label = document.createElement("div");
    label.className = "led-label";
    label.textContent = String(ledIdx);
    wrap.appendChild(circle);
    wrap.appendChild(label);
    statusLedsEl.appendChild(wrap);
  }
}

initLedGrid();

// ── Render ────────────────────────────────────────────────────────────────────
function renderStatus(frame: StatusFrame) {
  statusDisplay.style.display = "";

  // Battery
  const pct = frame.batteryPct;
  statusBatteryVal.textContent = `${pct}%`;
  statusBatteryBar.style.width = `${pct}%`;
  statusBatteryBar.style.background =
    pct > 50 ? "var(--success)" : pct > 20 ? "#e3b341" : "var(--error)";

  // WiFi
  statusWifiDot.className  = "dot " + (frame.wifiConnected ? "connected" : "disconnected");
  statusWifiText.textContent = frame.wifiConnected ? "Connected" : "Disconnected";
  statusRssi.textContent   = frame.wifiConnected ? `${frame.rssi} dBm` : "--";

  // GBFS fetch age
  if (frame.fetchAgeSecs === 0xffffffff) {
    statusFetchAge.textContent = "Never";
  } else {
    const s = frame.fetchAgeSecs;
    statusFetchAge.textContent = s < 60 ? `${s}s ago` : `${Math.floor(s / 60)}m ${s % 60}s ago`;
  }

  // Station input
  statusInput.textContent =
    STATION_NAMES[frame.stationInput] ?? `Unknown (${frame.stationInput})`;

  // LEDs
  for (let i = 0; i < 12; i++) {
    const el = document.getElementById(`led-${i}`);
    if (!el) continue;
    const { r, g, b } = frame.leds[i];
    const isOff = r === 0 && g === 0 && b === 0;
    el.style.backgroundColor = isOff ? "rgb(20,20,20)" : `rgb(${r},${g},${b})`;
    el.style.boxShadow = isOff ? "none" : `0 0 6px rgba(${r},${g},${b},0.6)`;
  }
}

// ── Status serial read loop ───────────────────────────────────────────────────
async function runStatusReadLoop(port: SerialPort) {
  const reader = port.readable!.getReader();
  statusReader = reader;
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      if (value) processStatusBytes(value);
    }
  } catch {
    // port closed / disconnected
  } finally {
    try { reader.releaseLock(); } catch { /* ignore */ }
    statusReader = null;
  }
}

async function releaseStatusPort() {
  if (statusReader) {
    try { await statusReader.cancel(); } catch { /* ignore */ }
    try { statusReader.releaseLock(); }  catch { /* ignore */ }
    statusReader = null;
  }
  if (statusDevice) {
    try { await statusDevice.close(); } catch { /* ignore */ }
    statusDevice = null;
  }
}

// ── Status connect/disconnect ─────────────────────────────────────────────────
statusConnectBtn.onclick = async () => {
  if (!("serial" in navigator)) {
    alert("Web Serial API is not supported. Use Chrome or Edge.");
    return;
  }
  try {
    await releaseStatusPort();
    statusDevice = await navigator.serial.requestPort();
    if (!statusDevice.readable) {
      await statusDevice.open({ baudRate: 115200 });
    }
    resetFramer();

    statusConnectBtn.style.display    = "none";
    statusDisconnectBtn.style.display = "inline";
    statusConnLabel.style.display     = "inline";
    statusConnLabel.textContent       = "Streaming…";

    runStatusReadLoop(statusDevice).then(() => {
      // Loop exited (port closed)
      statusConnectBtn.style.display    = "inline";
      statusDisconnectBtn.style.display = "none";
      statusConnLabel.style.display     = "none";
    });
  } catch (e) {
    if ((e as Error).name !== "NotAllowedError")
      alert("Could not open port: " + (e instanceof Error ? e.message : String(e)));
  }
};

statusDisconnectBtn.onclick = async () => {
  await releaseStatusPort();
  statusConnectBtn.style.display    = "inline";
  statusDisconnectBtn.style.display = "none";
  statusConnLabel.style.display     = "none";
  statusDisplay.style.display       = "none";
  resetFramer();
};
