// ── Frame protocol ────────────────────────────────────────────────────────────
//
// 49-byte binary frame emitted by the device every 500 ms:
//
// | Offset | Size | Field                                          |
// |--------|------|------------------------------------------------|
// |  0     |  1   | magic = 0xAB                                   |
// |  1     |  1   | battery_pct                                    |
// |  2     |  1   | wifi_connected (0/1)                           |
// |  3     |  1   | rssi (i8 bits)                                 |
// |  4     |  4   | fetch_age_secs LE  (0xFFFFFFFF = never)        |
// |  8     |  2   | station_input LE  (StationIdx ordinal as u16)  |
// | 10     |  2   | station_input_raw LE  (raw shift register u16) |
// | 12     | 36   | led_rgb — 12 × (r, g, b)                       |
// | 48     |  1   | XOR checksum of bytes 0–47                     |

import { STATION_IDS } from "./generated/station-ids";

const MAGIC      = 0xAB;
const FRAME_SIZE = 49;

// Fetches station names from the GBFS station_information feed (or the copy
// bundled into the deployment at /gbfs/station_information.json) and returns
// a map of station_id → human-readable name.
async function fetchStationNames(): Promise<Map<string, string>> {
  const urls = [
    // Bundled copy served alongside the web app (populated by CI).
    new URL("gbfs/station_information.json", window.location.href).href,
    // Live GBFS feed as fallback (also works in local dev).
    "https://gbfs.lyft.com/gbfs/2.3/bay/en/station_information.json",
  ];
  for (const url of urls) {
    try {
      const res = await fetch(url);
      if (!res.ok) continue;
      const json = await res.json() as {
        data: { stations: Array<{ station_id: string; name: string }> };
      };
      return new Map(json.data.stations.map(s => [s.station_id, s.name]));
    } catch { /* try next */ }
  }
  console.warn("status: GBFS station_information fetch failed; names unavailable");
  return new Map();
}

// Physical LED layout — top row: eBike LEDs 0-5, bottom row: mech LEDs 11-6 (reversed).
const LED_GRID_ORDER = [0, 1, 2, 3, 4, 5, 11, 10, 9, 8, 7, 6];

interface LedColor { r: number; g: number; b: number; }
interface StatusFrame {
  batteryPct:       number;
  wifiConnected:    boolean;
  rssi:             number;
  fetchAgeSecs:     number;
  stationInput:     number;
  stationInputRaw:  number;
  leds:             LedColor[];
}

function parseFrame(buf: Uint8Array): StatusFrame {
  const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  return {
    batteryPct:      buf[1],
    wifiConnected:   buf[2] === 1,
    rssi:            view.getInt8(3),
    fetchAgeSecs:    view.getUint32(4, true),
    stationInput:    view.getUint16(8, true),
    stationInputRaw: view.getUint16(10, true),
    leds: Array.from({ length: 12 }, (_, i) => ({
      r: buf[12 + i * 3],
      g: buf[12 + i * 3 + 1],
      b: buf[12 + i * 3 + 2],
    })),
  };
}

function checksumValid(buf: number[]): boolean {
  const xor = buf.slice(0, FRAME_SIZE - 1).reduce((a, b) => a ^ b, 0);
  return xor === buf[FRAME_SIZE - 1];
}

/** Wire up the Status tab. Self-contained — no shared state with other tabs. */
export function initStatusTab(): void {
  // Kick off the GBFS fetch immediately; render will use names once resolved.
  let stationIdToName = new Map<string, string>();
  fetchStationNames().then(m => { stationIdToName = m; });
  const connectBtn    = document.getElementById("statusConnectBtn")    as HTMLButtonElement;
  const disconnectBtn = document.getElementById("statusDisconnectBtn") as HTMLButtonElement;
  const connLabel     = document.getElementById("statusConnLabel")!;
  const display       = document.getElementById("statusDisplay")!;
  const batteryVal    = document.getElementById("statusBatteryVal")!;
  const batteryBar    = document.getElementById("statusBatteryBar")    as HTMLElement;
  const wifiDot       = document.getElementById("statusWifiDot")!;
  const wifiText      = document.getElementById("statusWifiText")!;
  const rssiEl        = document.getElementById("statusRssi")!;
  const fetchAgeEl    = document.getElementById("statusFetchAge")!;
  const inputEl       = document.getElementById("statusInput")!;
  const ledsEl        = document.getElementById("statusLeds")!;

  let statusDevice: SerialPort | null = null;
  let statusReader: ReadableStreamDefaultReader<Uint8Array> | null = null;
  let accumulator: number[] = [];

  // ── LED grid (rendered once at init) ─────────────────────────────────────────
  for (const idx of LED_GRID_ORDER) {
    const wrap   = document.createElement("div");
    const circle = document.createElement("div");
    circle.className       = "led-circle";
    circle.id              = `led-${idx}`;
    circle.style.backgroundColor = "rgb(20,20,20)";
    const label = document.createElement("div");
    label.className  = "led-label";
    label.textContent = String(idx);
    wrap.appendChild(circle);
    wrap.appendChild(label);
    ledsEl.appendChild(wrap);
  }

  // ── Frame framing ─────────────────────────────────────────────────────────────
  //
  // Accumulator-based: scan for 0xAB, read FRAME_SIZE bytes, verify checksum.
  // On failure discard one byte and re-scan — handles data bytes == 0xAB
  // (e.g. rssi == -85 dBm == 0xAB).
  function processBytes(bytes: Uint8Array) {
    for (const b of bytes) accumulator.push(b);

    while (accumulator.length >= FRAME_SIZE) {
      if (accumulator[0] !== MAGIC) { accumulator.shift(); continue; }

      if (checksumValid(accumulator)) {
        render(parseFrame(new Uint8Array(accumulator.slice(0, FRAME_SIZE))));
        accumulator.splice(0, FRAME_SIZE);
      } else {
        accumulator.shift(); // false magic byte — keep scanning
      }
    }
  }

  // ── Render ────────────────────────────────────────────────────────────────────
  function render(frame: StatusFrame) {
    display.style.display = "";

    // Battery
    const pct = frame.batteryPct;
    batteryVal.textContent    = `${pct}%`;
    batteryBar.style.width    = `${pct}%`;
    batteryBar.style.background =
      pct > 50 ? "var(--success)" : pct > 20 ? "#e3b341" : "var(--error)";

    // WiFi
    wifiDot.className      = "dot " + (frame.wifiConnected ? "connected" : "disconnected");
    wifiText.textContent   = frame.wifiConnected ? "Connected" : "Disconnected";
    rssiEl.textContent     = frame.wifiConnected ? `${frame.rssi} dBm` : "--";

    // GBFS fetch age
    if (frame.fetchAgeSecs === 0xffffffff) {
      fetchAgeEl.textContent = "Never";
    } else {
      const s = frame.fetchAgeSecs;
      fetchAgeEl.textContent = s < 60 ? `${s}s ago` : `${Math.floor(s / 60)}m ${s % 60}s ago`;
    }

    // Station input
    if (frame.stationInput === 255) {
      inputEl.textContent = "None";
    } else {
      const uuid = STATION_IDS[frame.stationInput];
      inputEl.textContent = uuid
        ? (stationIdToName.get(uuid) ?? uuid)
        : `Station ${frame.stationInput}`;
    }

    // LEDs
    for (let i = 0; i < 12; i++) {
      const el = document.getElementById(`led-${i}`);
      if (!el) continue;
      const { r, g, b } = frame.leds[i];
      const off = r === 0 && g === 0 && b === 0;
      el.style.backgroundColor = off ? "rgb(20,20,20)" : `rgb(${r},${g},${b})`;
      el.style.boxShadow       = off ? "none" : `0 0 6px rgba(${r},${g},${b},0.6)`;
    }
  }

  // ── Serial port management ────────────────────────────────────────────────────
  async function releasePort() {
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

  async function readLoop(port: SerialPort) {
    const reader = port.readable!.getReader();
    statusReader = reader;
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        if (value) processBytes(value);
      }
    } catch {
      // port closed or disconnected
    } finally {
      try { reader.releaseLock(); } catch { /* ignore */ }
      statusReader = null;
    }
  }

  function resetUi() {
    connectBtn.style.display    = "inline";
    disconnectBtn.style.display = "none";
    connLabel.style.display     = "none";
  }

  // ── Buttons ───────────────────────────────────────────────────────────────────
  connectBtn.onclick = async () => {
    if (!("serial" in navigator)) {
      alert("Web Serial API is not supported. Use Chrome or Edge.");
      return;
    }
    try {
      await releasePort();
      statusDevice = await navigator.serial.requestPort();
      if (!statusDevice.readable) {
        await statusDevice.open({ baudRate: 115200 });
      }
      accumulator = [];

      connectBtn.style.display    = "none";
      disconnectBtn.style.display = "inline";
      connLabel.style.display     = "inline";
      connLabel.textContent       = "Streaming…";

      readLoop(statusDevice).then(resetUi);
    } catch (e) {
      if ((e as Error).name !== "NotAllowedError")
        alert("Could not open port: " + (e instanceof Error ? e.message : String(e)));
    }
  };

  disconnectBtn.onclick = async () => {
    await releasePort();
    resetUi();
    display.style.display = "none";
    accumulator = [];
  };
}
