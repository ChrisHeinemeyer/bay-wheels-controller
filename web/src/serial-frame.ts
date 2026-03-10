// ── Frame protocol ────────────────────────────────────────────────────────────
//
// Version frame (34 bytes): sent once at startup. Magic 0xAC, 32-byte UTF-8 version, XOR checksum.
// Status frame (49 bytes): emitted every 500 ms. See src/tasks/serial_status.rs for layout.

export const MAGIC = 0xab;
export const FRAME_SIZE = 49;

export const VERSION_MAGIC = 0xac;
const VERSION_STR_LEN = 32;
export const VERSION_FRAME_SIZE = 1 + VERSION_STR_LEN + 1;

export interface StatusFrame {
  batteryPct: number;
  wifiConnected: boolean;
  rssi: number;
  fetchAgeSecs: number;
  stationInput: number;
  stationInputRaw: number;
  leds: Array<{ r: number; g: number; b: number }>;
}

export function parseFrame(buf: Uint8Array): StatusFrame {
  const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  return {
    batteryPct: buf[1],
    wifiConnected: buf[2] === 1,
    rssi: view.getInt8(3),
    fetchAgeSecs: view.getUint32(4, true),
    stationInput: view.getUint16(8, true),
    stationInputRaw: view.getUint16(10, true),
    leds: Array.from({ length: 12 }, (_, i) => ({
      r: buf[12 + i * 3],
      g: buf[12 + i * 3 + 1],
      b: buf[12 + i * 3 + 2],
    })),
  };
}

export function checksumValid(buf: number[]): boolean {
  const xor = buf.slice(0, FRAME_SIZE - 1).reduce((a, b) => a ^ b, 0);
  return xor === buf[FRAME_SIZE - 1];
}

export function parseVersionFrame(buf: Uint8Array): string | null {
  if (buf.length < VERSION_FRAME_SIZE || buf[0] !== VERSION_MAGIC) return null;
  const xor = buf.slice(0, VERSION_FRAME_SIZE - 1).reduce((a, b) => a ^ b, 0);
  if (xor !== buf[VERSION_FRAME_SIZE - 1]) return null;
  const versionBytes = buf.slice(1, 1 + VERSION_STR_LEN);
  const nullIdx = versionBytes.indexOf(0);
  const len = nullIdx >= 0 ? nullIdx : VERSION_STR_LEN;
  return new TextDecoder().decode(versionBytes.slice(0, len));
}

export function versionChecksumValid(buf: number[]): boolean {
  const xor = buf.slice(0, VERSION_FRAME_SIZE - 1).reduce((a, b) => a ^ b, 0);
  return xor === buf[VERSION_FRAME_SIZE - 1];
}
