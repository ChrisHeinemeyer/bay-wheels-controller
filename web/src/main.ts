import { ESPLoader, Transport, FlashOptions } from "esptool-js";

const CHUNK = 8192;
function uint8ArrayToBinaryString(arr: Uint8Array): string {
  let s = "";
  for (let i = 0; i < arr.length; i += CHUNK) {
    s += String.fromCharCode(...arr.subarray(i, i + CHUNK));
  }
  return s;
}

const FIRMWARE_URL =
  "https://github.com/ChrisHeinemeyer/bay-wheels-controller/releases/latest/download/firmware-bay-wheels-controller.bin";

const PROVISIONING_PROMPTS = {
  SSID: "Enter WiFi SSID: ",
  PASSWORD: "Enter WiFi Password: ",
  CONFIRM: "Save credentials? (y/n): ",
} as const;

type Step = "flash" | "wifi";

function log(element: HTMLElement, message: string, type?: "success" | "error") {
  const line = document.createElement("span");
  if (type) line.className = type;
  line.textContent = message + "\n";
  element.appendChild(line);
  element.scrollTop = element.scrollHeight;
}

function setStep(step: Step) {
  document.querySelectorAll(".step").forEach((el) => el.classList.remove("active"));
  document.querySelector(`.step[data-step="${step === "flash" ? 1 : 2}"]`)?.classList.add("active");
  document.querySelectorAll(".panel").forEach((el) => el.classList.remove("active"));
  document.getElementById(`panel-${step}`)?.classList.add("active");
}

function patchPortWithRetry(port: SerialPort): void {
  const nativeOpen = port.open.bind(port);
  const maxRetries = 3;
  port.open = async (options?: SerialOptions) => {
    const opts: SerialOptions = options ?? { baudRate: 115200 };
    let lastErr: unknown;
    for (let attempt = 0; attempt <= maxRetries; attempt++) {
      try {
        await nativeOpen(opts);
        return;
      } catch (err) {
        lastErr = err;
        const msg = err instanceof Error ? err.message.toLowerCase() : String(err).toLowerCase();
        const isRetryable = msg.includes("already open") || msg.includes("locked stream");
        if (!isRetryable || attempt === maxRetries) throw err;
        try {
          await port.close();
        } catch {
          // Ignore - port may have locked streams
        }
        await new Promise((r) => setTimeout(r, 1000 * (attempt + 1)));
      }
    }
    throw lastErr;
  };
}

function checkWebSerial() {
  if (!("serial" in navigator)) {
    alert(
      "Web Serial API is not supported in this browser. Please use Chrome or Edge on desktop."
    );
    return false;
  }
  return true;
}

async function getFirmwareData(): Promise<Uint8Array> {
  const source = (document.querySelector('input[name="firmware"]:checked') as HTMLInputElement)
    ?.value;
  const fileInput = document.getElementById("firmware-file") as HTMLInputElement;

  if (source === "file" && fileInput?.files?.length) {
    const file = fileInput.files[0];
    return new Uint8Array(await file.arrayBuffer());
  }

  const logEl = document.getElementById("flash-log")!;
  log(logEl, "Downloading firmware from GitHub...");
  const res = await fetch(FIRMWARE_URL);
  if (!res.ok) throw new Error(`Failed to download firmware: ${res.status}`);
  const buf = await res.arrayBuffer();
  log(logEl, `Downloaded ${(buf.byteLength / 1024).toFixed(1)} KB`);
  return new Uint8Array(buf);
}

async function runFlash() {
  if (!checkWebSerial()) return;

  const logEl = document.getElementById("flash-log")!;
  const progressEl = document.getElementById("flash-progress")!;
  const progressBar = progressEl.querySelector(".progress-bar") as HTMLElement;
  const progressText = progressEl.querySelector(".progress-text") as HTMLElement;
  const btn = document.getElementById("btn-flash") as HTMLButtonElement;

  logEl.innerHTML = "";
  progressEl.classList.remove("hidden");
  btn.disabled = true;

  try {
    for (const p of await navigator.serial.getPorts()) {
      try {
        await p.close();
      } catch {
        /* ignore */
      }
    }
    await new Promise((r) => setTimeout(r, 200));
    const port = await navigator.serial.requestPort();
    log(logEl, "Connecting to device...");

    patchPortWithRetry(port);
    const transport = new Transport(port);
    await transport.connect(115200);

    const terminal = {
      clean: () => {},
      write: (data: string) => log(logEl, data.trim()),
      writeLine: (data: string) => log(logEl, data),
    };

    const loader = new ESPLoader({
      transport,
      baudrate: 115200,
      romBaudrate: 115200,
      terminal,
    });

    await loader.connect();
    log(logEl, `Connected to ${loader.chip.CHIP_NAME}`);

    const firmwareData = await getFirmwareData();

    const options: FlashOptions = {
      fileArray: [{ address: 0, data: uint8ArrayToBinaryString(firmwareData) }],
      flashSize: "keep",
      flashMode: "keep",
      flashFreq: "keep",
      compress: true,
      eraseAll: true,
      reportProgress: (_fileIndex, written, total) => {
        const pct = total > 0 ? Math.round((written / total) * 100) : 0;
        progressBar.style.setProperty("--progress", `${pct}%`);
        progressText.textContent = `Flashing... ${pct}%`;
      },
    };

    await loader.writeFlash(options);
    log(logEl, "Firmware flashed successfully!", "success");

    await loader.after("hard_reset", true);
    await transport.disconnect();

    log(logEl, "Device is rebooting. You can now configure WiFi (Step 2).");
    setStep("wifi");
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    log(logEl, `Error: ${msg}`, "error");
  } finally {
    progressEl.classList.add("hidden");
    btn.disabled = false;
  }
}

async function runProvision() {
  if (!checkWebSerial()) return;

  const ssid = (document.getElementById("wifi-ssid") as HTMLInputElement).value.trim();
  const password = (document.getElementById("wifi-password") as HTMLInputElement).value.trim();
  const logEl = document.getElementById("wifi-log")!;
  const btn = document.getElementById("btn-provision") as HTMLButtonElement;

  if (!ssid) {
    log(logEl, "Please enter your WiFi network name.", "error");
    return;
  }

  logEl.innerHTML = "";
  btn.disabled = true;

  try {
    for (const p of await navigator.serial.getPorts()) {
      try {
        await p.close();
      } catch {
        /* ignore */
      }
    }
    await new Promise((r) => setTimeout(r, 200));
    const port = await navigator.serial.requestPort();
    log(logEl, "Connecting to device...");

    patchPortWithRetry(port);
    await port.open({ baudRate: 115200 });

    const encoder = new TextEncoder();
    const decoder = new TextDecoder();
    let buffer = "";

    const writer = port.writable!.getWriter();
    const reader = port.readable!.getReader();

    const send = (text: string) => {
      writer.write(encoder.encode(text + "\r\n"));
    };

    const readUntilPrompt = async (prompt: string) => {
      while (!buffer.includes(prompt)) {
        const { value, done } = await reader.read();
        if (done) throw new Error("Connection closed");
        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split(/\r?\n/);
        buffer = lines.pop() || "";
        for (const line of lines) {
          if (line.trim()) log(logEl, line);
        }
      }
    };

    await readUntilPrompt(PROVISIONING_PROMPTS.SSID);
    log(logEl, "Sending SSID...");
    send(ssid);

    await readUntilPrompt(PROVISIONING_PROMPTS.PASSWORD);
    log(logEl, "Sending password...");
    send(password);

    await readUntilPrompt(PROVISIONING_PROMPTS.CONFIRM);
    log(logEl, "Confirming save...");
    send("y");

    await new Promise((r) => setTimeout(r, 2000));
    const { value } = await reader.read();
    if (value) buffer += decoder.decode(value);
    if (buffer.includes("Credentials saved") || buffer.includes("successfully")) {
      log(logEl, "WiFi configured successfully! Device is rebooting.", "success");
    } else {
      log(logEl, "Provisioning complete. Check device output above.");
    }

    writer.releaseLock();
    reader.releaseLock();
    await port.close();
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    log(logEl, `Error: ${msg}`, "error");
  } finally {
    btn.disabled = false;
  }
}

document.getElementById("btn-flash")?.addEventListener("click", runFlash);
document.getElementById("btn-provision")?.addEventListener("click", runProvision);

document.querySelectorAll('input[name="firmware"]').forEach((radio) => {
  radio.addEventListener("change", () => {
    const fileInput = document.getElementById("file-input")!;
    fileInput.classList.toggle(
      "hidden",
      (document.querySelector('input[name="firmware"]:checked') as HTMLInputElement)?.value !==
        "file"
    );
  });
});
