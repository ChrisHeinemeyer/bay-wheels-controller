import L from "leaflet";
// ── Marker style presets ───────────────────────────────────────────────────
const S = {
  default: {
    fillColor: "#60a5fa",
    color: "#2563eb",
    radius: 5,
    weight: 1,
    fillOpacity: 0.85,
  },
  inBbox: {
    fillColor: "#a78bfa",
    color: "#7c3aed",
    radius: 6,
    weight: 1.5,
    fillOpacity: 0.9,
  },
  current: {
    fillColor: "#fbbf24",
    color: "#d97706",
    radius: 10,
    weight: 2,
    fillOpacity: 1,
  },
  mapped: {
    fillColor: "#34d399",
    color: "#059669",
    radius: 7,
    weight: 1.5,
    fillOpacity: 0.9,
  },
  skipped: {
    fillColor: "#6b7280",
    color: "#4b5563",
    radius: 5,
    weight: 1,
    fillOpacity: 0.45,
  },
};
export class MapView {
  map;
  markers = new Map();
  bboxGroup = L.layerGroup();
  constructor(containerId) {
    this.map = L.map(containerId, { preferCanvas: true }).setView(
      [37.77, -122.42],
      12,
    );
    L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", {
      attribution:
        '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a>',
      maxZoom: 19,
    }).addTo(this.map);
    this.bboxGroup.addTo(this.map);
  }
  // ── Station rendering ────────────────────────────────────────────────────
  plotStations(stations) {
    for (const st of stations) {
      const m = L.circleMarker([st.lat, st.lon], { ...S.default })
        .addTo(this.map)
        .bindTooltip(st.name, { direction: "top", offset: [0, -8] });
      this.markers.set(st.station_id, m);
    }
  }
  resetAll(stations) {
    for (const st of stations) {
      this.markers.get(st.station_id)?.setStyle({ ...S.default });
    }
  }
  highlightBboxStations(stations) {
    for (const st of stations) {
      this.markers.get(st.station_id)?.setStyle({ ...S.inBbox });
    }
  }
  setCurrentStation(station) {
    this.markers.get(station.station_id)?.setStyle({ ...S.current });
    this.map.panTo([station.lat, station.lon], {
      animate: true,
      duration: 0.4,
    });
  }
  setStationMapped(station, row, col) {
    const m = this.markers.get(station.station_id);
    if (!m) return;
    m.setStyle({ ...S.mapped });
    m.setTooltipContent(`[r${row} c${col}] ${station.name}`);
  }
  setStationSkipped(station) {
    this.markers.get(station.station_id)?.setStyle({ ...S.skipped });
  }
  // ── Bounding box ─────────────────────────────────────────────────────────
  /**
   * Enter interactive bbox-draw mode.  Returns a Promise that resolves with
   * the chosen BoundingBox when the user finishes drawing.
   */
  startBboxSelection() {
    return new Promise((resolve) => {
      const container = this.map.getContainer();
      container.style.cursor = "crosshair";
      this.map.dragging.disable();
      this.map.scrollWheelZoom.disable();
      this.bboxGroup.clearLayers();
      let startLatLng = null;
      let tempRect = null;
      const latLngFromEvent = (e) => {
        const rect = container.getBoundingClientRect();
        const pt = L.point(e.clientX - rect.left, e.clientY - rect.top);
        return this.map.containerPointToLatLng(pt);
      };
      const onMouseDown = (e) => {
        // Only handle left button
        if (e.button !== 0) return;
        startLatLng = latLngFromEvent(e);
        e.preventDefault();
      };
      const onMouseMove = (e) => {
        if (!startLatLng) return;
        const cur = latLngFromEvent(e);
        if (tempRect) this.bboxGroup.removeLayer(tempRect);
        const bounds = L.latLngBounds(startLatLng, cur);
        tempRect = L.rectangle(bounds, {
          color: "#f59e0b",
          fillColor: "#fef3c7",
          fillOpacity: 0.12,
          weight: 2,
          dashArray: "6 4",
        });
        this.bboxGroup.addLayer(tempRect);
      };
      const cleanup = () => {
        container.removeEventListener("mousedown", onMouseDown);
        document.removeEventListener("mousemove", onMouseMove);
        document.removeEventListener("mouseup", onMouseUp);
        container.style.cursor = "";
        this.map.dragging.enable();
        this.map.scrollWheelZoom.enable();
      };
      const onMouseUp = (e) => {
        if (!startLatLng) return;
        const end = latLngFromEvent(e);
        cleanup();
        if (tempRect) this.bboxGroup.removeLayer(tempRect);
        const bounds = L.latLngBounds(startLatLng, end);
        startLatLng = null;
        // Show the finalised bbox
        L.rectangle(bounds, {
          color: "#f59e0b",
          fillColor: "#fef3c7",
          fillOpacity: 0.06,
          weight: 2,
        }).addTo(this.bboxGroup);
        this.map.fitBounds(bounds, { padding: [30, 30] });
        resolve({
          north: bounds.getNorth(),
          south: bounds.getSouth(),
          east: bounds.getEast(),
          west: bounds.getWest(),
        });
      };
      container.addEventListener("mousedown", onMouseDown);
      document.addEventListener("mousemove", onMouseMove);
      document.addEventListener("mouseup", onMouseUp);
    });
  }
  showBbox(bbox) {
    this.bboxGroup.clearLayers();
    const bounds = L.latLngBounds(
      [bbox.south, bbox.west],
      [bbox.north, bbox.east],
    );
    L.rectangle(bounds, {
      color: "#f59e0b",
      fillColor: "#fef3c7",
      fillOpacity: 0.06,
      weight: 2,
    }).addTo(this.bboxGroup);
    this.map.fitBounds(bounds, { padding: [30, 30] });
  }
  clearBbox() {
    this.bboxGroup.clearLayers();
  }
}
