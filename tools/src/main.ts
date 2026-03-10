import "leaflet/dist/leaflet.css";
import "./style.css";

import { MapView } from "./map";
import { SerialConnection, isSinglePress, bitPosition } from "./serial";
import type { StatusFrame } from "./serial";
import { fetchStations, stationsInBbox, sortStationsForMapping } from "./gbfs";
import { buildSession, downloadYaml, promptLoadYaml } from "./yaml-io";
import type { GbfsStation, BoundingBox, StationMapping } from "./types";

// ── UI element helpers ────────────────────────────────────────────────────

function el<T extends HTMLElement>(id: string): T {
  const e = document.getElementById(id);
  if (!e) throw new Error(`#${id} not found`);
  return e as T;
}

// ── App ───────────────────────────────────────────────────────────────────

class BoardMapperApp {
  // State
  private allStations: GbfsStation[] = [];
  private bbox: BoundingBox | null = null;
  private queue: GbfsStation[] = []; // stations left to map in current run
  private mappings: StationMapping[] = [];
  private currentIdx = 0;

  // Resolvers for async station-flow control
  private inputResolver: ((n: number) => void) | null = null;
  private skipResolver: (() => void) | null = null;

  // Sub-systems
  private readonly mapView: MapView;
  private readonly serial: SerialConnection;

  // UI refs
  private readonly btnConnectSerial =
    el<HTMLButtonElement>("btn-connect-serial");
  private readonly btnLoadSession = el<HTMLButtonElement>("btn-load-session");
  private readonly btnSelectArea = el<HTMLButtonElement>("btn-select-area");
  private readonly btnStartMapping = el<HTMLButtonElement>("btn-start-mapping");
  private readonly btnExport = el<HTMLButtonElement>("btn-export");
  private readonly btnSkip = el<HTMLButtonElement>("btn-skip");
  private readonly btnUndo = el<HTMLButtonElement>("btn-undo");
  private readonly btnManualInput = el<HTMLButtonElement>("btn-manual-input");
  private readonly statusBadge = el<HTMLElement>("status-badge");
  private readonly serialBadge = el<HTMLElement>("serial-badge");
  private readonly currentPanel = el<HTMLElement>("current-panel");
  private readonly currentName = el<HTMLElement>("current-name");
  private readonly currentId = el<HTMLElement>("current-id");
  private readonly currentProgress = el<HTMLElement>("current-progress");
  private readonly serialLog = el<HTMLElement>("serial-log");
  private readonly mappingsList = el<HTMLElement>("mappings-list");
  private readonly stationCount = el<HTMLElement>("station-count");
  private readonly mappingCount = el<HTMLElement>("mapping-count");

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

  async init(): Promise<void> {
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

  private async toggleSerial(): Promise<void> {
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
        if ((e as Error).name !== "NotFoundError") {
          this.logSerial(`Connection error: ${e}`);
        }
      }
    }
  }

  private handleSerialFrame(frame: StatusFrame): void {
    const raw = frame.stationInputRaw;

    // Idle — nothing pressed
    if (raw === 0xffff) return;

    this.logSerial(`raw=0x${raw.toString(16).padStart(4, "0").toUpperCase()}`);

    if (!this.inputResolver) return;

    if (!isSinglePress(raw)) {
      this.logSerial(
        `⚠ Multi-button input (0x${raw.toString(16).toUpperCase()}) — release all and try again`,
      );
      return;
    }

    const bp = bitPosition(raw);
    const existing = this.mappings.find((m) => m.bit_position === bp);
    if (existing) {
      this.logSerial(
        `⚠ Bit ${bp} already mapped to "${existing.station_name}" — skipping`,
      );
      return;
    }

    const resolver = this.inputResolver;
    this.inputResolver = null;
    this.skipResolver = null;
    resolver(raw);
  }

  // ── Session load ─────────────────────────────────────────────────────────

  private async loadSession(): Promise<void> {
    const session = await promptLoadYaml();
    if (!session) return;

    this.mappings = session.mappings ?? [];
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
      if (st) this.mapView.setStationMapped(st, m.bit_position);
    }

    this.refreshMappingsList();
    this.setStatus(
      `Session loaded: ${this.mappings.length} mappings restored.`,
    );
  }

  // ── Area selection ───────────────────────────────────────────────────────

  private async selectArea(): Promise<void> {
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

  private async startMapping(): Promise<void> {
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

  private async runMappingLoop(): Promise<void> {
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

      const bp = bitPosition(result);
      const mapping: StationMapping = {
        bit_position: bp,
        raw_input: result,
        station_id: station.station_id,
        station_name: station.name,
        lat: station.lat,
        lon: station.lon,
      };
      this.mappings.push(mapping);
      this.mapView.setStationMapped(station, bp);
      this.logSerial(`✓ Mapped bit ${bp} (raw ${result}) → ${station.name}`);
      this.refreshMappingsList();
    }
  }

  private waitForInputOrSkip(): Promise<number | "skip"> {
    return new Promise((resolve) => {
      this.inputResolver = (n) => {
        this.skipResolver = null;
        resolve(n);
      };
      this.skipResolver = () => {
        this.inputResolver = null;
        resolve("skip");
      };
    });
  }

  private skipStation(): void {
    this.skipResolver?.();
    this.skipResolver = null;
  }

  private async promptManualInput(): Promise<void> {
    const input = prompt(
      "Enter raw u16 shift register value (decimal or hex 0x…):\n(e.g. 0xFFFE for bit 0, 0xFFFD for bit 1, 0x7FFF for bit 15)",
    );
    if (input === null) return;
    const trimmed = input.trim();
    if (!trimmed) return;
    const n =
      trimmed.startsWith("0x") || trimmed.startsWith("0X")
        ? parseInt(trimmed.slice(2), 16)
        : parseInt(trimmed, 10);
    if (isNaN(n) || n < 0 || n > 0xffff) {
      this.logSerial(`⚠ Invalid value: "${trimmed}"`);
      return;
    }
    this.handleSerialFrame({ stationInputRaw: n });
  }

  private undoLast(): void {
    if (this.mappings.length === 0) return;
    const last = this.mappings.pop()!;
    const st = this.allStations.find((s) => s.station_id === last.station_id);
    if (st) this.mapView.highlightBboxStations([st]);
    this.refreshMappingsList();
    this.logSerial(
      `↩ Undid mapping for "${last.station_name}" (bit ${last.bit_position})`,
    );

    // Put the station back in front of the queue so we revisit it
    const inQueue = this.queue.some((s) => s.station_id === last.station_id);
    if (!inQueue && st) {
      this.queue.splice(this.currentIdx, 0, st);
    }
  }

  // ── Export ───────────────────────────────────────────────────────────────

  private exportYaml(): void {
    const session = buildSession(this.mappings, this.bbox);
    downloadYaml(session);
  }

  // ── UI helpers ────────────────────────────────────────────────────────────

  private setStatus(msg: string, error = false): void {
    this.statusBadge.textContent = msg;
    this.statusBadge.classList.toggle("badge--error", error);
  }

  private logSerial(line: string): void {
    const entry = document.createElement("div");
    entry.className = "log-entry";
    entry.textContent = `[${new Date().toLocaleTimeString()}] ${line}`;
    this.serialLog.prepend(entry);
    // Keep log from growing unboundedly
    while (this.serialLog.children.length > 80) {
      this.serialLog.lastElementChild?.remove();
    }
  }

  private refreshMappingsList(): void {
    this.mappingCount.textContent = String(this.mappings.length);
    this.btnExport.disabled = this.mappings.length === 0;
    this.btnUndo.disabled = this.mappings.length === 0;

    this.mappingsList.innerHTML = "";
    const sorted = [...this.mappings].sort(
      (a, b) => a.bit_position - b.bit_position,
    );
    for (const m of sorted) {
      const row = document.createElement("div");
      row.className = "mapping-row";
      row.innerHTML = `
        <span class="bit-badge">bit ${m.bit_position}</span>
        <span class="station-info">
          <span class="station-name">${m.station_name}</span>
          <span class="station-id">${m.station_id}</span>
        </span>
        <span class="raw-val">${m.raw_input}</span>
      `;
      this.mappingsList.appendChild(row);
    }
  }
}

// ── Bootstrap ─────────────────────────────────────────────────────────────
const app = new BoardMapperApp();
void app.init();
