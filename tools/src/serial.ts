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
// | 10     |  2   | station_input_raw LE  (raw shift register u16) |
// | 12     | 36   | led_rgb — 12 × (r, g, b)                       |
// | 48     |  1   | XOR checksum of bytes 0–47                     |

const MAGIC      = 0xAB;
const FRAME_SIZE = 49;

/** Parsed subset of the status frame relevant to the board-mapper tool. */
export interface StatusFrame {
  /** Raw 16-bit shift-register reading (active-low; 0xFFFF = idle). */
  stationInputRaw: number;
}

export type FrameHandler = (frame: StatusFrame) => void;

export class SerialConnection {
  private port: SerialPort | null = null;
  private abortController: AbortController | null = null;
  private frameHandler: FrameHandler | null = null;
  // Rolling byte buffer for framing
  private rxBuf: number[] = [];

  static isSupported(): boolean {
    return 'serial' in navigator;
  }

  async connect(baudRate = 115200): Promise<void> {
    this.port = await navigator.serial.requestPort();
    await this.port.open({ baudRate });
    this.abortController = new AbortController();
    void this.readLoop(this.abortController.signal);
  }

  private async readLoop(signal: AbortSignal): Promise<void> {
    if (!this.port?.readable) return;
    const reader = (this.port.readable as ReadableStream<Uint8Array>).getReader();
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

  private processBuffer(): void {
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

  onFrame(handler: FrameHandler): void {
    this.frameHandler = handler;
  }

  async disconnect(): Promise<void> {
    this.abortController?.abort();
    this.abortController = null;
    try { await this.port?.close(); } catch { /* ignore */ }
    this.port = null;
    this.rxBuf = [];
  }

  get isConnected(): boolean {
    return this.port !== null;
  }
}

function checksumValid(frame: number[]): boolean {
  const xor = frame.slice(0, FRAME_SIZE - 1).reduce((a, b) => a ^ b, 0);
  return xor === frame[FRAME_SIZE - 1];
}

function parseFrame(frame: number[]): StatusFrame {
  const buf = new Uint8Array(frame);
  const view = new DataView(buf.buffer);
  return {
    stationInputRaw: view.getUint16(10, /* littleEndian */ true),
  };
}

/**
 * Return true if exactly one bit is low (one button pressed) in an
 * active-low 16-bit shift register reading.
 * 0xFFFF means idle (no button pressed).
 */
export function isSinglePress(raw: number): boolean {
  const active = (~raw) & 0xFFFF;
  return active !== 0 && (active & (active - 1)) === 0;
}

/**
 * Return the 0-indexed bit position (0 = MSB, 15 = LSB) that is active-low
 * in the raw shift-register reading. Only meaningful when `isSinglePress` is true.
 *
 * This matches the firmware convention: bit position 0 corresponds to bit 15
 * of the u16 (the first bit shifted out).
 */
export function bitPosition(raw: number): number {
  const active = (~raw) & 0xFFFF;
  return 15 - Math.clz32(active);
}
