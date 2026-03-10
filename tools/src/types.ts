export interface GbfsStation {
  station_id: string;
  name: string;
  lat: number;
  lon: number;
  capacity?: number;
  short_name?: string;
}

export interface BoundingBox {
  north: number;
  south: number;
  east: number;
  west: number;
}

export interface StationMapping {
  /** 0-indexed bit position derived from raw_input (log2). */
  bit_position: number;
  /** Raw integer value received from the shift register. */
  raw_input: number;
  station_id: string;
  station_name: string;
  lat: number;
  lon: number;
}

export interface SavedSession {
  created_at: string;
  bounding_box: BoundingBox | null;
  mappings: StationMapping[];
}

// ── Web Serial API type augmentations ──────────────────────────────────────
// Not included in the standard TypeScript lib yet; defining minimal shapes.
export interface SerialPortFilter {
  usbVendorId?: number;
  usbProductId?: number;
}

declare global {
  interface Navigator {
    readonly serial: Serial;
  }

  interface Serial extends EventTarget {
    requestPort(options?: {
      filters?: SerialPortFilter[];
    }): Promise<SerialPort>;
    getPorts(): Promise<SerialPort[]>;
  }

  interface SerialPort extends EventTarget {
    open(options: SerialOptions): Promise<void>;
    close(): Promise<void>;
    readonly readable: ReadableStream<Uint8Array> | null;
    readonly writable: WritableStream<Uint8Array> | null;
  }

  interface SerialOptions {
    baudRate: number;
    dataBits?: number;
    stopBits?: number;
    parity?: "none" | "even" | "odd";
    bufferSize?: number;
    flowControl?: "none" | "hardware";
  }
}
