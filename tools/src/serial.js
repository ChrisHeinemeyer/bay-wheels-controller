// ── Frame protocol (matches serial_status.rs) ─────────────────────────────────
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
// | 10     |  1   | station_input_row (0xFF = idle)               |
// | 11     |  1   | station_input_col (0xFF = idle)               |
// | 12     | 36   | led_rgb — 12 × (r, g, b)                       |
// | 48     |  1   | XOR checksum of bytes 0–47                     |
const MAGIC = 0xab;
const FRAME_SIZE = 49;
export class SerialConnection {
  port = null;
  abortController = null;
  frameHandler = null;
  // Rolling byte buffer for framing
  rxBuf = [];
  static isSupported() {
    return "serial" in navigator;
  }
  async connect(baudRate = 115200) {
    this.port = await navigator.serial.requestPort();
    await this.port.open({ baudRate });
    this.abortController = new AbortController();
    void this.readLoop(this.abortController.signal);
  }
  async readLoop(signal) {
    if (!this.port?.readable) return;
    const reader = this.port.readable.getReader();
    try {
      while (!signal.aborted) {
        const { value, done } = await reader.read();
        if (done) break;
        for (const byte of value) {
          this.rxBuf.push(byte);
        }
        this.processBuffer();
      }
    } catch {
      // Connection lost or aborted
    } finally {
      reader.releaseLock();
    }
  }
  processBuffer() {
    // Scan for a valid frame, advancing past any false magic bytes.
    while (this.rxBuf.length >= FRAME_SIZE) {
      const magicIdx = this.rxBuf.indexOf(MAGIC);
      if (magicIdx === -1) {
        this.rxBuf = [];
        return;
      }
      // Discard bytes before the magic
      if (magicIdx > 0) {
        this.rxBuf.splice(0, magicIdx);
      }
      if (this.rxBuf.length < FRAME_SIZE) return;
      const frame = this.rxBuf.slice(0, FRAME_SIZE);
      if (checksumValid(frame)) {
        this.rxBuf.splice(0, FRAME_SIZE);
        this.frameHandler?.(parseFrame(frame));
      } else {
        // False magic byte — skip it and keep scanning
        this.rxBuf.splice(0, 1);
      }
    }
  }
  onFrame(handler) {
    this.frameHandler = handler;
  }
  async disconnect() {
    this.abortController?.abort();
    this.abortController = null;
    try {
      await this.port?.close();
    } catch {
      /* ignore */
    }
    this.port = null;
    this.rxBuf = [];
  }
  get isConnected() {
    return this.port !== null;
  }
}
function checksumValid(frame) {
  const xor = frame.slice(0, FRAME_SIZE - 1).reduce((a, b) => a ^ b, 0);
  return xor === frame[FRAME_SIZE - 1];
}
function parseFrame(frame) {
  return {
    stationInputRow: frame[10],
    stationInputCol: frame[11],
  };
}
/** Return true if exactly one (row, col) input is active (0xFF = idle). */
export function isSinglePress(row, col) {
  return row !== 0xff && col !== 0xff;
}
