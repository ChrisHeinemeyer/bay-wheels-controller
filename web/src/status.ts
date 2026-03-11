import "leaflet/dist/leaflet.css";
import L from "leaflet";

import { STATION_IDS } from "./generated/station-ids";
import {
  MAGIC,
  FRAME_SIZE,
  VERSION_MAGIC,
  VERSION_FRAME_SIZE,
  parseFrame,
  parseVersionFrame,
  checksumValid,
  versionChecksumValid,
} from "./serial-frame";

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

// Matches firmware: row 1 = eBikes (1–5) + Led11, row 2 = mechanical (10,9,8,7,0), row 3 = empty (6, red)
const LED_ROW_1 = [1, 2, 3, 4, 5];
const LED_ROW_2 = [10, 9, 8, 7, 0];
const LED_ROW_3 = [6];

// Maki marker SVG (https://github.com/mapbox/maki/blob/main/icons/marker.svg)
// Single closed path — solid fill. iconAnchor = bottom center, pin tip at station coordinate
const MARKER_SVG_PATH =
  "M7.5 1C5.42312 1 3 2.2883 3 5.56759C3 7.79276 6.46156 12.7117 7.5 14C8.42309 12.7117 12 7.90993 12 5.56759C12 2.2883 9.57688 1 7.5 1Z";

const BASE_SIZE = 24;

function createMarkerIcon(
  fillColor: string,
  strokeColor: string,
  className: string,
  scale: number,
): L.DivIcon {
  const size = Math.round(BASE_SIZE * scale);
  const anchorX = size / 2;
  const anchorY = size;
  return L.divIcon({
    className: `station-marker-icon ${className}`,
    html: `<svg width="${size}" height="${size}" viewBox="0 0 15 15" xmlns="http://www.w3.org/2000/svg"><path fill="${fillColor}" stroke="${strokeColor}" stroke-width="1" d="${MARKER_SVG_PATH}"/></svg>`,
    iconSize: [size, size],
    iconAnchor: [anchorX, anchorY],
  });
}

const MARKER_ICON_DEFAULT = createMarkerIcon(
  "#C4DCF9",
  "#8BADF7",
  "station-marker-default",
  0.8,
);
const MARKER_ICON_HIGHLIGHT = createMarkerIcon(
  "#FF00BFFF",
  "#FF00BFFF",
  "station-marker-highlight",
  1.2,
);

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
  const versionEl = document.getElementById("statusVersion")!;
  const boardIdEl = document.getElementById("statusBoardId")!;
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
  const markers = new Map<string, L.Marker>();
  let currentHighlight: string | null = null;
  const targetIds = new Set(Object.values(STATION_IDS));

  function setMapHighlight(stationId: string | null) {
    if (currentHighlight === stationId) return;
    if (currentHighlight) {
      markers.get(currentHighlight)?.setIcon(MARKER_ICON_DEFAULT);
    }
    currentHighlight = stationId;
    if (stationId) {
      markers.get(stationId)?.setIcon(MARKER_ICON_HIGHLIGHT);
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
        const m = L.marker([st.lat, st.lon], { icon: MARKER_ICON_DEFAULT })
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

  // ── LED grid (rendered once at init, 3 explicit rows) ────────────────────────
  for (const rowLeds of [LED_ROW_1, LED_ROW_2, LED_ROW_3]) {
    const rowEl = document.createElement("div");
    rowEl.className = "led-row";
    for (const idx of rowLeds) {
      const wrap = document.createElement("div");
      const circle = document.createElement("div");
      circle.className = "led-circle";
      circle.id = `led-${idx}`;
      circle.style.backgroundColor = "rgb(20,20,20)";
      wrap.appendChild(circle);
      rowEl.appendChild(wrap);
    }
    ledsEl.appendChild(rowEl);
  }

  function processBytes(bytes: Uint8Array) {
    for (const b of bytes) accumulator.push(b);
    while (accumulator.length >= Math.min(FRAME_SIZE, VERSION_FRAME_SIZE)) {
      if (
        accumulator[0] === VERSION_MAGIC &&
        accumulator.length >= VERSION_FRAME_SIZE
      ) {
        if (versionChecksumValid(accumulator)) {
          const version = parseVersionFrame(
            new Uint8Array(accumulator.slice(0, VERSION_FRAME_SIZE)),
          );
          if (version) versionEl.textContent = version;
          accumulator.splice(0, VERSION_FRAME_SIZE);
        } else {
          accumulator.shift();
        }
      } else if (accumulator[0] === MAGIC && accumulator.length >= FRAME_SIZE) {
        if (checksumValid(accumulator)) {
          render(parseFrame(new Uint8Array(accumulator.slice(0, FRAME_SIZE))));
          accumulator.splice(0, FRAME_SIZE);
        } else {
          accumulator.shift();
        }
      } else {
        accumulator.shift();
      }
    }
  }

  const BOARD_ID_LABELS = ["Board 0", "Board 1", "Board 2", "Board 3"] as const;

  function render(frame: ReturnType<typeof parseFrame>) {
    display.style.display = "";

    // Board ID
    boardIdEl.textContent =
      frame.boardId < 4
        ? BOARD_ID_LABELS[frame.boardId]
        : `Board ${frame.boardId}`;

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
      versionEl.textContent = "--";
      boardIdEl.textContent = "--";

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
    versionEl.textContent = "--";
    boardIdEl.textContent = "--";
  };

  (
    window as Window & { initStatusDisplayReady?: () => void }
  ).initStatusDisplayReady = () => initMap();
}
