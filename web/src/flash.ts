import { ESPLoader, Transport, FlashOptions } from "esptool-js";
import { extractFirmwareVersion } from "./firmware-version";
import {
  VERSION_MAGIC,
  VERSION_FRAME_SIZE,
  MAGIC,
  FRAME_SIZE,
  parseVersionFrame,
  versionChecksumValid,
  checksumValid,
} from "./serial-frame";

// GitHub repo for fetching latest release tag (owner/repo).
const GITHUB_REPO = "ChrisHeinemeyer/bay-wheels-controller";

// Always fetch firmware directly from GitHub releases so users get the latest.
const FIRMWARE_URL =
  "https://github.com/ChrisHeinemeyer/bay-wheels-controller/releases/latest/download/firmware-bay-wheels-controller.bin";

const CHUNK = 8192;
function uint8ArrayToBinaryString(arr: Uint8Array): string {
  let s = "";
  for (let i = 0; i < arr.length; i += CHUNK)
    s += String.fromCharCode(...arr.subarray(i, i + CHUNK));
  return s;
}

/**
 * Wire up the Flash tab.
 * @param onFlashComplete Called after a successful flash so the orchestrator
 *                        can set `justFlashed` and switch to the WiFi tab.
 */
export function initFlashTab(onFlashComplete: () => void): void {
  const connectButton = document.getElementById(
    "connectButton",
  ) as HTMLButtonElement;
  const disconnectButton = document.getElementById(
    "disconnectButton",
  ) as HTMLButtonElement;
  const programButton = document.getElementById(
    "programButton",
  ) as HTMLButtonElement;
  const flashOptions = document.getElementById("flashOptions")!;
  const lblConnTo = document.getElementById("lblConnTo")!;
  const terminal = document.getElementById("terminal")!;
  const firmwareFile = document.getElementById(
    "firmwareFile",
  ) as HTMLInputElement;
  const flashLatestVersion = document.getElementById("flashLatestVersion")!;
  const flashFileVersion = document.getElementById("flashFileVersion")!;

  let device: SerialPort | null = null;
  let transport: Transport | null = null;
  let esploader: ESPLoader | null = null;
  let chipShort = "";
  let deviceVersion: string | null = null;
  let latestReleaseVersion: string | null = null;
  let fileVersion: string | null = null;
  let flashReader: ReadableStreamDefaultReader<Uint8Array> | null = null;

  function updateConnectionLabel() {
    const chip = chipShort || "…";
    const version =
      deviceVersion ??
      ((
        document.querySelector(
          'input[name="firmware"]:checked',
        ) as HTMLInputElement
      )?.value === "file"
        ? fileVersion
        : latestReleaseVersion);
    lblConnTo.textContent =
      "Connected: " + chip + (version ? ` — ${version}` : "");
  }

  document.querySelectorAll('input[name="firmware"]').forEach((el) => {
    el.addEventListener("change", updateConnectionLabel);
  });

  function log(msg: string) {
    terminal.textContent += msg + "\n";
    terminal.scrollTop = terminal.scrollHeight;
  }
  function logClear() {
    terminal.textContent = "";
  }

  const espLoaderTerminal = {
    clean: () => logClear(),
    writeLine: (data: string) => log(data),
    write: (data: string) => {
      terminal.textContent += data;
      terminal.scrollTop = terminal.scrollHeight;
    },
  };

  function processBytes(accumulator: number[]) {
    while (accumulator.length >= Math.min(FRAME_SIZE, VERSION_FRAME_SIZE)) {
      if (
        accumulator[0] === VERSION_MAGIC &&
        accumulator.length >= VERSION_FRAME_SIZE
      ) {
        if (versionChecksumValid(accumulator)) {
          const version = parseVersionFrame(
            new Uint8Array(accumulator.slice(0, VERSION_FRAME_SIZE)),
          );
          if (version) {
            deviceVersion = version;
            updateConnectionLabel();
          }
          accumulator.splice(0, VERSION_FRAME_SIZE);
        } else {
          accumulator.shift();
        }
      } else if (accumulator[0] === MAGIC && accumulator.length >= FRAME_SIZE) {
        if (checksumValid(accumulator)) {
          accumulator.splice(0, FRAME_SIZE);
        } else {
          accumulator.shift();
        }
      } else {
        accumulator.shift();
      }
    }
  }

  async function flashReadLoop(port: SerialPort) {
    await port.open({ baudRate: 115200 });
    const readable = port.readable;
    if (!readable) return;
    const reader = readable.getReader();
    flashReader = reader;
    const accumulator: number[] = [];
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        if (value) for (const b of value) accumulator.push(b);
        processBytes(accumulator);
      }
    } catch {
      /* port closed */
    } finally {
      try {
        reader.releaseLock();
      } catch {
        /* ignore */
      }
      flashReader = null;
    }
  }

  async function stopFlashRead(closePort = false) {
    if (flashReader) {
      try {
        await flashReader.cancel();
      } catch {
        /* ignore */
      }
      try {
        flashReader.releaseLock();
      } catch {
        /* ignore */
      }
      flashReader = null;
    }
    if (closePort && device) {
      try {
        await device.close();
      } catch {
        /* ignore */
      }
    }
  }

  // ── Connect ──────────────────────────────────────────────────────────────────
  connectButton.onclick = async () => {
    if (!("serial" in navigator)) {
      alert("Web Serial API is not supported. Use Chrome or Edge.");
      return;
    }
    try {
      device = await navigator.serial.requestPort();
      deviceVersion = null;
      chipShort = "";
      lblConnTo.textContent = "Connected: …";
      lblConnTo.style.display = "inline";
      connectButton.style.display = "none";
      disconnectButton.style.display = "inline";
      flashOptions.style.display = "block";
      flashReadLoop(device).catch(() => {});
      fetchLatestReleaseVersion().then((v) => {
        latestReleaseVersion = v;
        flashLatestVersion.textContent = v ? `(${v})` : "";
        updateConnectionLabel();
      });
    } catch (e) {
      log("Error: " + (e instanceof Error ? e.message : String(e)));
    }
  };

  // ── Disconnect ───────────────────────────────────────────────────────────────
  disconnectButton.onclick = async () => {
    if (transport) await transport.disconnect();
    await stopFlashRead(true);
    logClear();
    connectButton.style.display = "inline";
    disconnectButton.style.display = "none";
    lblConnTo.style.display = "none";
    flashOptions.style.display = "none";
    flashLatestVersion.textContent = "";
    flashFileVersion.textContent = "";
    chipShort = "";
    deviceVersion = null;
    latestReleaseVersion = null;
    fileVersion = null;
    device = null;
    transport = null;
    esploader = null;
  };

  async function fetchLatestReleaseVersion(): Promise<string | null> {
    try {
      const res = await fetch(
        `https://api.github.com/repos/${GITHUB_REPO}/releases/latest`,
      );
      if (!res.ok) return null;
      const json = (await res.json()) as { tag_name?: string };
      return json.tag_name ?? null;
    } catch {
      return null;
    }
  }

  firmwareFile.onchange = async () => {
    flashFileVersion.textContent = "";
    fileVersion = null;
    const file = firmwareFile.files?.[0];
    if (!file) return;
    try {
      const buf = await file.arrayBuffer();
      const version = extractFirmwareVersion(new Uint8Array(buf));
      fileVersion = version;
      flashFileVersion.textContent = version ? `— ${version}` : "";
      updateConnectionLabel();
    } catch {
      /* ignore */
    }
  };

  // ── Program ──────────────────────────────────────────────────────────────────
  programButton.onclick = async () => {
    if (!device) return;

    await stopFlashRead(true);
    transport = new Transport(device);
    esploader = new ESPLoader({
      transport,
      baudrate: 921600,
      romBaudrate: 115200,
      terminal: espLoaderTerminal,
    });
    const chip = await esploader.main();
    chipShort = chip.split(" (")[0] ?? chip;
    updateConnectionLabel();

    const source = (
      document.querySelector(
        'input[name="firmware"]:checked',
      ) as HTMLInputElement
    )?.value;
    let firmwareData: Uint8Array;

    if (source === "file" && firmwareFile.files?.length) {
      firmwareData = new Uint8Array(await firmwareFile.files[0].arrayBuffer());
    } else {
      log("Downloading firmware from latest release...");
      const res = await fetch(FIRMWARE_URL);
      if (!res.ok)
        throw new Error("Failed to download firmware: " + res.status);
      firmwareData = new Uint8Array(await res.arrayBuffer());
      log(`Downloaded ${(firmwareData.length / 1024).toFixed(1)} KB`);
    }

    const progressEl = document.getElementById("progress0")!;
    try {
      programButton.disabled = true;
      const options: FlashOptions = {
        fileArray: [
          { address: 0x0, data: uint8ArrayToBinaryString(firmwareData) },
        ],
        flashSize: "keep",
        flashMode: "keep",
        flashFreq: "keep",
        eraseAll: true,
        compress: true,
        reportProgress: (_i, written, total) => {
          progressEl.textContent =
            total > 0 ? Math.round((written / total) * 100) + "%" : "";
        },
      };
      await esploader.writeFlash(options);
      await esploader.after("hard_reset", true);
      log("Firmware flashed! Switching to WiFi setup...");

      // The hard reset causes USB re-enumeration — drop the handle entirely.
      try {
        await transport!.disconnect();
      } catch {
        /* ignore */
      }
      transport = null;
      esploader = null;
      try {
        await device!.close();
      } catch {
        /* ignore */
      }
      device = null;

      onFlashComplete();
    } catch (e) {
      log("Error: " + (e instanceof Error ? e.message : String(e)));
    } finally {
      programButton.disabled = false;
      progressEl.textContent = "";
    }
  };
}
