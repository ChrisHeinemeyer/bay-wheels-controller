// ── Frame protocol ────────────────────────────────────────────────────────────
//
// 49-byte binary frame emitted by the device every 500 ms.
// See src/tasks/serial_status.rs for the device-side layout.

export const MAGIC = 0xab;
export const FRAME_SIZE = 49;

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
