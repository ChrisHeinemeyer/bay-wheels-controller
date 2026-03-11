import "leaflet/dist/leaflet.css";
import "./style.css";
import { MapView } from "./map";
import { SerialConnection, isSinglePress } from "./serial";
import { fetchStations, stationsInBbox, sortStationsForMapping } from "./gbfs";
import { buildSession, downloadYaml, promptLoadYaml } from "./yaml-io";
// ── UI element helpers ────────────────────────────────────────────────────
function el(id) {
  const e = document.getElementById(id);
  if (!e) throw new Error(`#${id} not found`);
  return e;
}
// ── App ───────────────────────────────────────────────────────────────────
class BoardMapperApp {
  // State
  allStations = [];
  bbox = null;
  queue = []; // stations left to map in current run
  mappings = [];
  currentIdx = 0;
  // Resolvers for async station-flow control
  inputResolver = null;
  skipResolver = null;
  // Sub-systems
  mapView;
  serial;
  // UI refs
  btnConnectSerial = el("btn-connect-serial");
  btnLoadSession = el("btn-load-session");
  btnSelectArea = el("btn-select-area");
  btnStartMapping = el("btn-start-mapping");
  btnExport = el("btn-export");
  btnSkip = el("btn-skip");
  btnUndo = el("btn-undo");
  btnManualInput = el("btn-manual-input");
  statusBadge = el("status-badge");
  serialBadge = el("serial-badge");
  currentPanel = el("current-panel");
  currentName = el("current-name");
  currentId = el("current-id");
  currentProgress = el("current-progress");
  serialLog = el("serial-log");
  mappingsList = el("mappings-list");
  stationCount = el("station-count");
  mappingCount = el("mapping-count");
  constructor() {
    this.mapView = new MapView("map");
    this.serial = new SerialConnection();
    this.serial.onFrame((frame) => this.handleSerialFrame(frame));
    this.btnConnectSerial.addEventListener(
      "click",
      () => void this.toggleSerial(),
    );
    this.btnLoadSession.addEventListener(
      "click",
      () => void this.loadSession(),
    );
    this.btnSelectArea.addEventListener("click", () => void this.selectArea());
    this.btnStartMapping.addEventListener(
      "click",
      () => void this.startMapping(),
    );
    this.btnExport.addEventListener("click", () => this.exportYaml());
    this.btnSkip.addEventListener("click", () => this.skipStation());
    this.btnUndo.addEventListener("click", () => this.undoLast());
    this.btnManualInput.addEventListener(
      "click",
      () => void this.promptManualInput(),
    );
    if (!SerialConnection.isSupported()) {
      this.serialBadge.textContent =
        "Serial: Not supported in this browser (use Chrome/Edge)";
      this.serialBadge.classList.add("badge--warn");
      this.btnConnectSerial.disabled = true;
    }
  }
  async init() {
    this.setStatus("Loading stations…");
    try {
      this.allStations = await fetchStations();
      this.mapView.plotStations(this.allStations);
      this.setStatus(
        `Loaded ${this.allStations.length} stations. Draw a bounding box to begin.`,
      );
      this.btnSelectArea.disabled = false;
      this.btnLoadSession.disabled = false;
    } catch (e) {
      this.setStatus(`Error: ${e}`, true);
    }
  }
  // ── Serial ───────────────────────────────────────────────────────────────
  async toggleSerial() {
    if (this.serial.isConnected) {
      await this.serial.disconnect();
      this.btnConnectSerial.textContent = "Connect Serial";
      this.serialBadge.textContent = "Serial: Disconnected";
      this.serialBadge.classList.remove("badge--ok");
    } else {
      try {
        await this.serial.connect();
        this.btnConnectSerial.textContent = "Disconnect Serial";
        this.serialBadge.textContent = "Serial: Connected";
        this.serialBadge.classList.add("badge--ok");
      } catch (e) {
        if (e.name !== "NotFoundError") {
          this.logSerial(`Connection error: ${e}`);
        }
      }
    }
  }
  handleSerialFrame(frame) {
    const { stationInputRow: row, stationInputCol: col } = frame;
    // Idle — nothing pressed
    if (row === 0xff && col === 0xff) return;
    this.logSerial(`raw=r${row} c${col}`);
    if (!this.inputResolver) return;
    if (!isSinglePress(row, col)) {
      this.logSerial(
        `⚠ Invalid input (r${row} c${col}) — release all and try again`,
      );
      return;
    }
    const existing = this.mappings.find(
      (m) => m.row === row && m.column === col,
    );
    if (existing) {
      this.logSerial(
        `⚠ [r${row} c${col}] already mapped to "${existing.station_name}" — skipping`,
      );
      return;
    }
    const resolver = this.inputResolver;
    this.inputResolver = null;
    this.skipResolver = null;
    resolver({ row, col });
  }
  // ── Session load ─────────────────────────────────────────────────────────
  migrateMapping(m) {
    const o = m;
    if (!o || typeof o !== "object") return null;
    // New format
    if ("row" in o && "column" in o) {
      return o;
    }
    // Legacy: bit_position 0-15 → row=0, col=bit_position
    if ("bit_position" in o && typeof o.bit_position === "number") {
      return {
        row: 0,
        column: o.bit_position,
        station_id: String(o.station_id ?? ""),
        station_name: String(o.station_name ?? ""),
        lat: Number(o.lat ?? 0),
        lon: Number(o.lon ?? 0),
      };
    }
    return null;
  }
  async loadSession() {
    const session = await promptLoadYaml();
    if (!session) return;
    const raw = session.mappings ?? [];
    this.mappings = raw
      .map((m) => this.migrateMapping(m))
      .filter((m) => m !== null);
    if (session.bounding_box) {
      this.bbox = session.bounding_box;
      this.mapView.showBbox(this.bbox);
      const inBox = stationsInBbox(this.allStations, this.bbox);
      this.mapView.highlightBboxStations(inBox);
      this.stationCount.textContent = String(inBox.length);
      this.btnStartMapping.disabled = false;
    }
    // Render previously-recorded mappings on the map
    for (const m of this.mappings) {
      const st = this.allStations.find((s) => s.station_id === m.station_id);
      if (st) this.mapView.setStationMapped(st, m.row, m.column);
    }
    this.refreshMappingsList();
    this.setStatus(
      `Session loaded: ${this.mappings.length} mappings restored.`,
    );
  }
  // ── Area selection ───────────────────────────────────────────────────────
  async selectArea() {
    this.setStatus("Draw a rectangle on the map to select your area…");
    this.btnSelectArea.disabled = true;
    // Reset any previous bbox highlights
    this.mapView.resetAll(this.allStations);
    this.mapView.clearBbox();
    this.bbox = await this.mapView.startBboxSelection();
    const filtered = stationsInBbox(this.allStations, this.bbox);
    this.stationCount.textContent = String(filtered.length);
    this.mapView.highlightBboxStations(filtered);
    this.btnSelectArea.disabled = false;
    this.btnStartMapping.disabled = filtered.length === 0;
    this.setStatus(
      filtered.length > 0
        ? `${filtered.length} stations in area. Connect serial and click Start Mapping.`
        : "No stations in selected area. Try a different area.",
    );
  }
  // ── Mapping flow ─────────────────────────────────────────────────────────
  async startMapping() {
    if (!this.bbox) return;
    const inBox = stationsInBbox(this.allStations, this.bbox);
    const alreadyMapped = new Set(this.mappings.map((m) => m.station_id));
    const remaining = sortStationsForMapping(
      inBox.filter((s) => !alreadyMapped.has(s.station_id)),
    );
    if (remaining.length === 0) {
      this.setStatus("All stations in this area are already mapped!");
      return;
    }
    this.queue = remaining;
    this.currentIdx = 0;
    this.btnStartMapping.disabled = true;
    this.btnSelectArea.disabled = true;
    this.btnLoadSession.disabled = true;
    this.btnSkip.disabled = false;
    this.btnManualInput.disabled = false;
    this.currentPanel.hidden = false;
    await this.runMappingLoop();
    this.btnStartMapping.disabled = false;
    this.btnSelectArea.disabled = false;
    this.btnLoadSession.disabled = false;
    this.btnSkip.disabled = true;
    this.btnManualInput.disabled = true;
    this.currentPanel.hidden = true;
    this.setStatus(
      `Done! ${this.mappings.length} total mappings. Export YAML when ready.`,
    );
    this.btnExport.disabled = this.mappings.length === 0;
  }
  async runMappingLoop() {
    const total = this.queue.length;
    for (let i = 0; i < this.queue.length; i++) {
      const station = this.queue[i];
      this.currentIdx = i;
      // Update UI
      this.currentName.textContent = station.name;
      this.currentId.textContent = station.station_id;
      this.currentProgress.textContent = `${i + 1} / ${total}`;
      this.setStatus(`Touch the pad for: ${station.name}`);
      this.mapView.setCurrentStation(station);
      this.btnUndo.disabled = this.mappings.length === 0;
      const result = await this.waitForInputOrSkip();
      if (result === "skip") {
        this.mapView.setStationSkipped(station);
        this.logSerial(`— Skipped: ${station.name}`);
        continue;
      }
      const { row, col } = result;
      const mapping = {
        row,
        column: col,
        station_id: station.station_id,
        station_name: station.name,
        lat: station.lat,
        lon: station.lon,
      };
      this.mappings.push(mapping);
      this.mapView.setStationMapped(station, row, col);
      this.logSerial(`✓ Mapped [r${row} c${col}] → ${station.name}`);
      this.refreshMappingsList();
    }
  }
  waitForInputOrSkip() {
    return new Promise((resolve) => {
      this.inputResolver = (pos) => {
        this.skipResolver = null;
        resolve(pos);
      };
      this.skipResolver = () => {
        this.inputResolver = null;
        resolve("skip");
      };
    });
  }
  skipStation() {
    this.skipResolver?.();
    this.skipResolver = null;
  }
  async promptManualInput() {
    const input = prompt("Enter row,column (e.g. 0,5 or 2,3):");
    if (input === null) return;
    const trimmed = input.trim();
    if (!trimmed) return;
    const parts = trimmed.split(/[,\s]+/);
    if (parts.length !== 2) {
      this.logSerial(`⚠ Enter row,column (e.g. 0,5)`);
      return;
    }
    const row = parseInt(parts[0], 10);
    const col = parseInt(parts[1], 10);
    if (
      isNaN(row) ||
      isNaN(col) ||
      row < 0 ||
      row > 0xff ||
      col < 0 ||
      col > 0xff
    ) {
      this.logSerial(`⚠ Invalid row,column: "${trimmed}"`);
      return;
    }
    this.handleSerialFrame({ stationInputRow: row, stationInputCol: col });
  }
  undoLast() {
    if (this.mappings.length === 0) return;
    const last = this.mappings.pop();
    const st = this.allStations.find((s) => s.station_id === last.station_id);
    if (st) this.mapView.highlightBboxStations([st]);
    this.refreshMappingsList();
    this.logSerial(
      `↩ Undid mapping for "${last.station_name}" [r${last.row} c${last.column}]`,
    );
    // Put the station back in front of the queue so we revisit it
    const inQueue = this.queue.some((s) => s.station_id === last.station_id);
    if (!inQueue && st) {
      this.queue.splice(this.currentIdx, 0, st);
    }
  }
  // ── Export ───────────────────────────────────────────────────────────────
  exportYaml() {
    const session = buildSession(this.mappings, this.bbox);
    downloadYaml(session);
  }
  // ── UI helpers ────────────────────────────────────────────────────────────
  setStatus(msg, error = false) {
    this.statusBadge.textContent = msg;
    this.statusBadge.classList.toggle("badge--error", error);
  }
  logSerial(line) {
    const entry = document.createElement("div");
    entry.className = "log-entry";
    entry.textContent = `[${new Date().toLocaleTimeString()}] ${line}`;
    this.serialLog.prepend(entry);
    // Keep log from growing unboundedly
    while (this.serialLog.children.length > 80) {
      this.serialLog.lastElementChild?.remove();
    }
  }
  refreshMappingsList() {
    this.mappingCount.textContent = String(this.mappings.length);
    this.btnExport.disabled = this.mappings.length === 0;
    this.btnUndo.disabled = this.mappings.length === 0;
    this.mappingsList.innerHTML = "";
    const sorted = [...this.mappings].sort(
      (a, b) => a.row - b.row || a.column - b.column,
    );
    for (const m of sorted) {
      const row = document.createElement("div");
      row.className = "mapping-row";
      row.innerHTML = `
        <span class="bit-badge">r${m.row} c${m.column}</span>
        <span class="station-info">
          <span class="station-name">${m.station_name}</span>
          <span class="station-id">${m.station_id}</span>
        </span>
        <span class="raw-val">r${m.row} c${m.column}</span>
      `;
      this.mappingsList.appendChild(row);
    }
  }
}
// ── Bootstrap ─────────────────────────────────────────────────────────────
const app = new BoardMapperApp();
void app.init();
