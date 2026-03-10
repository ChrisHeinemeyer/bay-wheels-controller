import "leaflet/dist/leaflet.css";
import L from "leaflet";

import { STATION_IDS } from "./generated/station-ids";
import { MAGIC, FRAME_SIZE, parseFrame, checksumValid } from "./serial-frame";

const NONE_OR_UNKNOWN = new Set([255, 65535]);

interface GbfsStation {
  station_id: string;
  name: string;
  lat: number;
  lon: number;
}

async function fetchStationNames(): Promise<Map<string, string>> {
  const urls = [
    new URL("gbfs/station_information.json", window.location.href).href,
    "https://gbfs.lyft.com/gbfs/2.3/bay/en/station_information.json",
  ];
  for (const url of urls) {
    try {
      const res = await fetch(url);
      if (!res.ok) continue;
      const json = (await res.json()) as {
        data: { stations: Array<{ station_id: string; name: string }> };
      };
      return new Map(json.data.stations.map((s) => [s.station_id, s.name]));
    } catch {
      /* try next */
    }
  }
  console.warn(
    "status: GBFS station_information fetch failed; names unavailable",
  );
  return new Map();
}

async function fetchStationsWithCoords(): Promise<GbfsStation[]> {
  const urls = [
    new URL("gbfs/station_information.json", window.location.href).href,
    "https://gbfs.lyft.com/gbfs/2.3/bay/en/station_information.json",
  ];
  for (const url of urls) {
    try {
      const res = await fetch(url);
      if (!res.ok) continue;
      const json = (await res.json()) as {
        data: {
          stations: Array<{
            station_id: string;
            name: string;
            lat: number;
            lon: number;
          }>;
        };
      };
      return json.data.stations;
    } catch {
      /* try next */
    }
  }
  throw new Error("Could not fetch GBFS station_information");
}

const LED_GRID_ORDER = [0, 1, 2, 3, 4, 5, 11, 10, 9, 8, 7, 6];

const MAP_DEFAULT_STYLE = {
  fillColor: "#60a5fa",
  color: "#2563eb",
  radius: 3,
  weight: 1,
  fillOpacity: 0.5,
};
const MAP_HIGHLIGHT_STYLE = {
  fillColor: "#DE24FB",
  color: "#661174",
  radius: 15,
  weight: 2,
  fillOpacity: 1,
};

/** Wire up the Status tab. Self-contained — no shared state with other tabs. */
export function initStatusTab(): void {
  let stationIdToName = new Map<string, string>();
  fetchStationNames().then((m) => {
    stationIdToName = m;
  });

  const connectBtn = document.getElementById(
    "statusConnectBtn",
  ) as HTMLButtonElement;
  const disconnectBtn = document.getElementById(
    "statusDisconnectBtn",
  ) as HTMLButtonElement;
  const connLabel = document.getElementById("statusConnLabel")!;
  const display = document.getElementById("statusDisplay")!;
  const batteryVal = document.getElementById("statusBatteryVal")!;
  const batteryBar = document.getElementById("statusBatteryBar") as HTMLElement;
  const wifiDot = document.getElementById("statusWifiDot")!;
  const wifiText = document.getElementById("statusWifiText")!;
  const rssiEl = document.getElementById("statusRssi")!;
  const fetchAgeEl = document.getElementById("statusFetchAge")!;
  const inputOverlay = document.getElementById("statusInputOverlay")!;
  const ledsEl = document.getElementById("statusLeds")!;

  let statusDevice: SerialPort | null = null;
  let statusReader: ReadableStreamDefaultReader<Uint8Array> | null = null;
  let accumulator: number[] = [];

  let map: L.Map | null = null;
  const markers = new Map<string, L.CircleMarker>();
  let currentHighlight: string | null = null;
  const targetIds = new Set(Object.values(STATION_IDS));

  function setMapHighlight(stationId: string | null) {
    if (currentHighlight === stationId) return;
    if (currentHighlight) {
      markers.get(currentHighlight)?.setStyle(MAP_DEFAULT_STYLE);
    }
    currentHighlight = stationId;
    if (stationId) {
      markers.get(stationId)?.setStyle(MAP_HIGHLIGHT_STYLE);
      const m = markers.get(stationId);
      // if (m) map?.panTo(m.getLatLng(), { animate: true, duration: 0.3 });
    }
  }

  async function initMap() {
    if (map) return;
    try {
      const allStations = await fetchStationsWithCoords();
      const ourStations = allStations.filter((s) =>
        targetIds.has(s.station_id),
      );

      map = L.map("statusMapContainer", { preferCanvas: true });
      L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", {
        attribution:
          '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a>',
        maxZoom: 19,
      }).addTo(map);

      for (const st of ourStations) {
        const m = L.circleMarker([st.lat, st.lon], { ...MAP_DEFAULT_STYLE })
          .addTo(map!)
          .bindTooltip(st.name, { direction: "top", offset: [0, -8] });
        markers.set(st.station_id, m);
      }

      const bounds = L.latLngBounds(
        [37.721712, -122.538125], // southwest (bottom-left)
        [37.815441, -122.345146], // northeast (top-right)
      );
      map.fitBounds(bounds, { padding: [20, 20], maxZoom: 14 });
      map.invalidateSize();
    } catch (e) {
      console.warn("status: map init failed", e);
    }
  }

  // ── LED grid (rendered once at init) ─────────────────────────────────────────
  for (const idx of LED_GRID_ORDER) {
    const wrap = document.createElement("div");
    const circle = document.createElement("div");
    circle.className = "led-circle";
    circle.id = `led-${idx}`;
    circle.style.backgroundColor = "rgb(20,20,20)";
    const label = document.createElement("div");
    label.className = "led-label";
    label.textContent = String(idx);
    wrap.appendChild(circle);
    wrap.appendChild(label);
    ledsEl.appendChild(wrap);
  }

  function processBytes(bytes: Uint8Array) {
    for (const b of bytes) accumulator.push(b);
    while (accumulator.length >= FRAME_SIZE) {
      if (accumulator[0] !== MAGIC) {
        accumulator.shift();
        continue;
      }
      if (checksumValid(accumulator)) {
        render(parseFrame(new Uint8Array(accumulator.slice(0, FRAME_SIZE))));
        accumulator.splice(0, FRAME_SIZE);
      } else {
        accumulator.shift();
      }
    }
  }

  function render(frame: ReturnType<typeof parseFrame>) {
    display.style.display = "";

    // Battery
    const pct = frame.batteryPct;
    batteryVal.textContent = `${pct}%`;
    batteryBar.style.width = `${pct}%`;
    batteryBar.style.background =
      pct > 50 ? "var(--success)" : pct > 20 ? "#e3b341" : "var(--error)";

    // WiFi
    wifiDot.className =
      "dot " + (frame.wifiConnected ? "connected" : "disconnected");
    wifiText.textContent = frame.wifiConnected ? "Connected" : "Disconnected";
    rssiEl.textContent = frame.wifiConnected ? `${frame.rssi} dBm` : "--";

    // GBFS fetch age
    if (frame.fetchAgeSecs === 0xffffffff) {
      fetchAgeEl.textContent = "Never";
    } else {
      const s = frame.fetchAgeSecs;
      fetchAgeEl.textContent =
        s < 60 ? `${s}s ago` : `${Math.floor(s / 60)}m ${s % 60}s ago`;
    }

    // Station input (overlay + map highlight)
    if (NONE_OR_UNKNOWN.has(frame.stationInput)) {
      inputOverlay.textContent = "None";
      setMapHighlight(null);
    } else {
      const uuid = STATION_IDS[frame.stationInput];
      const label = uuid
        ? (stationIdToName.get(uuid) ?? uuid)
        : `Station ${frame.stationInput}`;
      inputOverlay.textContent = label;
      if (uuid) setMapHighlight(uuid);
    }

    // LEDs
    for (let i = 0; i < 12; i++) {
      const el = document.getElementById(`led-${i}`);
      if (!el) continue;
      const { r, g, b } = frame.leds[i];
      const off = r === 0 && g === 0 && b === 0;
      el.style.backgroundColor = off ? "rgb(20,20,20)" : `rgb(${r},${g},${b})`;
      el.style.boxShadow = off ? "none" : `0 0 6px rgba(${r},${g},${b},0.6)`;
    }
  }

  async function releasePort() {
    if (statusReader) {
      try {
        await statusReader.cancel();
      } catch {
        /* ignore */
      }
      try {
        statusReader.releaseLock();
      } catch {
        /* ignore */
      }
      statusReader = null;
    }
    if (statusDevice) {
      try {
        await statusDevice.close();
      } catch {
        /* ignore */
      }
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
      /* port closed */
    } finally {
      try {
        reader.releaseLock();
      } catch {
        /* ignore */
      }
      statusReader = null;
    }
  }

  function resetUi() {
    connectBtn.style.display = "inline";
    disconnectBtn.style.display = "none";
    connLabel.style.display = "none";
  }

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
      setMapHighlight(null);

      connectBtn.style.display = "none";
      disconnectBtn.style.display = "inline";
      connLabel.style.display = "inline";
      connLabel.textContent = "Streaming…";
      display.classList.remove("status-disconnected");

      initMap();
      readLoop(statusDevice).then(resetUi);
    } catch (e) {
      if ((e as Error).name !== "NotAllowedError") {
        alert(
          "Could not open port: " +
            (e instanceof Error ? e.message : String(e)),
        );
      }
    }
  };

  disconnectBtn.onclick = async () => {
    await releasePort();
    resetUi();
    display.classList.add("status-disconnected");
    accumulator = [];
    setMapHighlight(null);
  };

  (
    window as Window & { initStatusDisplayReady?: () => void }
  ).initStatusDisplayReady = () => initMap();
}
