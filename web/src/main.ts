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
function showTab(name: "flash" | "wifi") {
  flashTab.style.display = name === "flash" ? "" : "none";
  wifiTab.style.display  = name === "wifi"  ? "" : "none";
  tabButtons.forEach(btn => btn.classList.toggle("active", btn.dataset.tab === name));
  if (name === "wifi") setupWifiTab();
}

tabButtons.forEach(btn => {
  btn.addEventListener("click", () => showTab(btn.dataset.tab as "flash" | "wifi"));
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
