import "leaflet/dist/leaflet.css";
import L from "leaflet";

import { STATION_IDS } from "./generated/station-ids";
import { MAGIC, FRAME_SIZE, parseFrame, checksumValid } from "./serial-frame";

interface GbfsStation {
  station_id: string;
  name: string;
  lat: number;
  lon: number;
}

const NONE_OR_UNKNOWN = new Set([255, 65535]);

async function fetchStations(): Promise<GbfsStation[]> {
  const urls = [
    new URL("gbfs/station_information.json", window.location.href).href,
    "https://gbfs.lyft.com/gbfs/2.3/bay/en/station_information.json",
  ];
  for (const url of urls) {
    try {
      const res = await fetch(url);
      if (!res.ok) continue;
      const json = (await res.json()) as {
        data: { stations: Array<{ station_id: string; name: string; lat: number; lon: number }> };
      };
      return json.data.stations;
    } catch {
      /* try next */
    }
  }
  throw new Error("Could not fetch GBFS station_information");
}

const DEFAULT_STYLE = {
  fillColor: "#60a5fa",
  color: "#2563eb",
  radius: 6,
  weight: 1,
  fillOpacity: 0.85,
};
const HIGHLIGHT_STYLE = {
  fillColor: "#fbbf24",
  color: "#d97706",
  radius: 12,
  weight: 2,
  fillOpacity: 1,
};

/** Wire up the Map tab: serial listener + map with station highlighting. */
export function initMapTab(): void {
  const connectBtn = document.getElementById("mapConnectBtn") as HTMLButtonElement;
  const disconnectBtn = document.getElementById("mapDisconnectBtn") as HTMLButtonElement;
  const connLabel = document.getElementById("mapConnLabel")!;
  const statusEl = document.getElementById("mapStatus")!;

  let map: L.Map | null = null;
  const markers = new Map<string, L.CircleMarker>();
  let currentHighlight: string | null = null;
  let mapDevice: SerialPort | null = null;
  let mapReader: ReadableStreamDefaultReader<Uint8Array> | null = null;
  let accumulator: number[] = [];

  function setHighlight(stationId: string | null) {
    if (currentHighlight === stationId) return;
    if (currentHighlight) {
      markers.get(currentHighlight)?.setStyle(DEFAULT_STYLE);
    }
    currentHighlight = stationId;
    if (stationId) {
      markers.get(stationId)?.setStyle(HIGHLIGHT_STYLE);
      const m = markers.get(stationId);
      if (m) map?.panTo(m.getLatLng(), { animate: true, duration: 0.3 });
    }
  }

  function processBytes(bytes: Uint8Array) {
    for (const b of bytes) accumulator.push(b);
    while (accumulator.length >= FRAME_SIZE) {
      if (accumulator[0] !== MAGIC) {
        accumulator.shift();
        continue;
      }
      if (checksumValid(accumulator)) {
        const frame = parseFrame(new Uint8Array(accumulator.slice(0, FRAME_SIZE)));
        accumulator.splice(0, FRAME_SIZE);
        const ord = frame.stationInput;
        if (NONE_OR_UNKNOWN.has(ord)) {
          setHighlight(null);
        } else {
          const uuid = STATION_IDS[ord];
          if (uuid) setHighlight(uuid);
        }
      } else {
        accumulator.shift();
      }
    }
  }

  async function releasePort() {
    if (mapReader) {
      try {
        await mapReader.cancel();
      } catch {
        /* ignore */
      }
      try {
        mapReader.releaseLock();
      } catch {
        /* ignore */
      }
      mapReader = null;
    }
    if (mapDevice) {
      try {
        await mapDevice.close();
      } catch {
        /* ignore */
      }
      mapDevice = null;
    }
  }

  async function readLoop(port: SerialPort) {
    const reader = port.readable!.getReader();
    mapReader = reader;
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
      mapReader = null;
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
      mapDevice = await navigator.serial.requestPort();
      if (!mapDevice.readable) {
        await mapDevice.open({ baudRate: 115200 });
      }
      accumulator = [];
      setHighlight(null);

      connectBtn.style.display = "none";
      disconnectBtn.style.display = "inline";
      connLabel.style.display = "inline";
      connLabel.textContent = "Streaming…";

      readLoop(mapDevice).then(resetUi);
    } catch (e) {
      if ((e as Error).name !== "NotAllowedError") {
        alert("Could not open port: " + (e instanceof Error ? e.message : String(e)));
      }
    }
  };

  disconnectBtn.onclick = async () => {
    await releasePort();
    resetUi();
    setHighlight(null);
  };

  // Build the set of station IDs we care about (from our firmware's TARGET_STATIONS)
  const targetIds = new Set(Object.values(STATION_IDS));

  // Fetch stations and init map when tab is first shown
  let initialized = false;
  const doInit = async () => {
    if (initialized) return;
    initialized = true;
    statusEl.textContent = "Loading stations…";
    try {
      const allStations = await fetchStations();
      const ourStations = allStations.filter((s) => targetIds.has(s.station_id));
      statusEl.textContent = `${ourStations.length} stations on map`;

      map = L.map("mapContainer", { preferCanvas: true }).setView([37.77, -122.42], 11);
      L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", {
        attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a>',
        maxZoom: 19,
      }).addTo(map);

      for (const st of ourStations) {
        const m = L.circleMarker([st.lat, st.lon], { ...DEFAULT_STYLE })
          .addTo(map!)
          .bindTooltip(st.name, { direction: "top", offset: [0, -8] });
        markers.set(st.station_id, m);
      }
    } catch (e) {
      statusEl.textContent = "Failed to load stations: " + (e instanceof Error ? e.message : String(e));
      statusEl.style.color = "var(--error)";
    }
  };

  // Expose doInit so main.ts can call it when the map tab is activated
  (window as Window & { initMapTabReady?: () => Promise<void> }).initMapTabReady = doInit;
}
