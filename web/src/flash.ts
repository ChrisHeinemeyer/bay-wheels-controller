import { ESPLoader, Transport, FlashOptions } from "esptool-js";

// Resolves relative to the current page so it works in both local dev
// (Vite proxy) and on GitHub Pages.
const FIRMWARE_URL = new URL(
  "firmware-bay-wheels-controller.bin",
  window.location.href,
).href;

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

  let device: SerialPort | null = null;
  let transport: Transport | null = null;
  let esploader: ESPLoader | null = null;

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

  // ── Connect ──────────────────────────────────────────────────────────────────
  connectButton.onclick = async () => {
    if (!("serial" in navigator)) {
      alert("Web Serial API is not supported. Use Chrome or Edge.");
      return;
    }
    try {
      device = await navigator.serial.requestPort();
      transport = new Transport(device);
      esploader = new ESPLoader({
        transport,
        baudrate: 921600,
        romBaudrate: 115200,
        terminal: espLoaderTerminal,
      });
      const chip = await esploader.main();

      lblConnTo.textContent = "Connected: " + chip;
      lblConnTo.style.display = "inline";
      connectButton.style.display = "none";
      disconnectButton.style.display = "inline";
      flashOptions.style.display = "block";
    } catch (e) {
      log("Error: " + (e instanceof Error ? e.message : String(e)));
    }
  };

  // ── Disconnect ───────────────────────────────────────────────────────────────
  disconnectButton.onclick = async () => {
    if (transport) await transport.disconnect();
    logClear();
    connectButton.style.display = "inline";
    disconnectButton.style.display = "none";
    lblConnTo.style.display = "none";
    flashOptions.style.display = "none";
    device = null;
    transport = null;
    esploader = null;
  };

  // ── Program ──────────────────────────────────────────────────────────────────
  programButton.onclick = async () => {
    if (!esploader) return;

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
