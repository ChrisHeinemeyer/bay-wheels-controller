/**
 * Wire up the WiFi / provisioning tab.
 * @param isJustFlashed Getter so the tab can show the correct instruction text.
 * @returns `onActivate` — call this whenever the WiFi tab becomes visible.
 */
export function initWifiTab(isJustFlashed: () => boolean): () => void {
  const wifiInstructions = document.getElementById("wifiInstructions")!;
  const wifiInstructionsText = document.getElementById("wifiInstructionsText")!;
  const wifiConnectBtn = document.getElementById(
    "wifiConnectBtn",
  ) as HTMLButtonElement;
  const ssidInput = document.getElementById("ssidInput") as HTMLInputElement;
  const passwordInput = document.getElementById(
    "passwordInput",
  ) as HTMLInputElement;
  const wifiConfigBtn = document.getElementById(
    "wifiConfigBtn",
  ) as HTMLButtonElement;
  const wifiStatus = document.getElementById("wifiStatus")!;
  const wifiSpinner = document.getElementById("wifiSpinner")!;
  const wifiStatusIcon = document.getElementById("wifiStatusIcon")!;
  const wifiStatusText = document.getElementById("wifiStatusText")!;

  let device: SerialPort | null = null;
  let activeReader: ReadableStreamDefaultReader<Uint8Array> | null = null;

  async function releasePort() {
    if (activeReader) {
      try {
        await activeReader.cancel();
      } catch {
        /* ignore */
      }
      try {
        activeReader.releaseLock();
      } catch {
        /* ignore */
      }
      activeReader = null;
    }
    if (device) {
      try {
        await device.close();
      } catch {
        /* ignore */
      }
      device = null;
    }
  }

  // ── Status indicator ─────────────────────────────────────────────────────────
  type WifiStatusState = "idle" | "working" | "success" | "error";

  function setWifiStatus(state: WifiStatusState, message = "") {
    wifiStatus.style.display = state === "idle" ? "none" : "flex";
    wifiSpinner.style.display = state === "working" ? "inline-block" : "none";
    wifiStatusIcon.style.display =
      state === "success" || state === "error" ? "inline" : "none";
    wifiStatusText.textContent = message;

    wifiConfigBtn.disabled = state === "working";
    ssidInput.disabled = state === "working";
    passwordInput.disabled = state === "working";

    if (state === "success") {
      wifiStatusIcon.textContent = "✓";
      wifiStatusIcon.className = "status-icon success";
    } else if (state === "error") {
      wifiStatusIcon.textContent = "✗";
      wifiStatusIcon.className = "status-icon error";
    } else {
      wifiStatusIcon.className = "status-icon";
    }
  }

  // ── Tab activation ───────────────────────────────────────────────────────────
  function onActivate() {
    setWifiStatus("idle");
    if (!device) {
      if (isJustFlashed()) {
        wifiInstructionsText.textContent =
          "Firmware flashed! Unplug the device, plug it back in, then click Connect.";
      } else {
        wifiInstructionsText.innerHTML =
          "Hold the <strong>GPIO2</strong> button while power cycling your board " +
          "to enter WiFi setup mode, then click Connect.";
      }
      wifiInstructions.style.display = "block";
      wifiConnectBtn.style.display = "inline-block";
    } else {
      wifiInstructions.style.display = "none";
    }
  }

  // ── Connect button (WiFi tab) ─────────────────────────────────────────────────
  wifiConnectBtn.onclick = async () => {
    if (!("serial" in navigator)) {
      alert("Web Serial API is not supported. Use Chrome or Edge.");
      return;
    }
    try {
      device = await navigator.serial.requestPort();
      wifiInstructions.style.display = "none";
    } catch (e) {
      if ((e as Error).name !== "NotAllowedError")
        alert(
          "Could not open port: " +
            (e instanceof Error ? e.message : String(e)),
        );
    }
  };

  // ── Configure button ──────────────────────────────────────────────────────────
  wifiConfigBtn.onclick = async () => {
    const ssid = ssidInput.value.trim();
    if (!ssid) {
      ssidInput.focus();
      return;
    }
    if (!device) {
      alert("Connect to your device first.");
      return;
    }

    setWifiStatus("working", "Connecting...");
    try {
      if (!device.readable) {
        await device.open({ baudRate: 115200 });
      }
      await runProvisioning(
        device,
        ssid,
        passwordInput.value,
        (msg, done, isError) => {
          if (done) setWifiStatus("success", msg);
          else if (isError) setWifiStatus("error", msg);
          else setWifiStatus("working", msg);
        },
      );
    } catch (e) {
      setWifiStatus("error", e instanceof Error ? e.message : String(e));
    } finally {
      await releasePort();
    }
  };

  // Submit on Enter from either credential field
  [ssidInput, passwordInput].forEach((input) => {
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") wifiConfigBtn.click();
    });
  });

  // ── Provisioning state machine ────────────────────────────────────────────────
  function provLog(msg: string) {
    console.log(`[provisioning] ${msg}`);
  }

  async function runProvisioning(
    port: SerialPort,
    ssid: string,
    password: string,
    onStatus: (msg: string, done?: boolean, error?: boolean) => void,
  ): Promise<void> {
    const dec = new TextDecoder();
    const enc = new TextEncoder();
    let buf = "";
    type State = "ssid" | "pwd" | "confirm" | "result";
    let state: State = "ssid";

    async function send(text: string) {
      provLog(`→ sending: ${text === password ? "***" : JSON.stringify(text)}`);
      const writer = port.writable!.getWriter();
      // Firmware's drain_newline() reads one extra byte after \r or \n.
      // Send \r\n so drain_newline consumes the \n and doesn't block.
      try {
        await writer.write(enc.encode(text + "\r\n"));
      } finally {
        writer.releaseLock();
      }
    }

    async function ensureOpen() {
      provLog(`port.readable=${port.readable != null}`);
      if (!port.readable) {
        provLog("opening port at 115200...");
        await port.open({ baudRate: 115200 });
        provLog("port opened");
      }
    }

    const deadline = Date.now() + 120_000; // 2 min overall timeout
    provLog("starting (state=ssid)");
    onStatus("Waiting for device...");

    await ensureOpen();
    let reader = port.readable!.getReader();
    activeReader = reader;
    provLog("reader acquired, entering read loop");

    try {
      while (Date.now() < deadline) {
        let value: Uint8Array | undefined;

        try {
          // Race the read against a 5 s timeout so we don't block forever
          const raceResult = await Promise.race([
            reader.read() as Promise<ReadableStreamReadResult<Uint8Array>>,
            new Promise<ReadableStreamReadResult<Uint8Array>>((resolve) =>
              setTimeout(
                () => resolve({ value: new Uint8Array(0), done: false }),
                5_000,
              ),
            ),
          ]);
          if (raceResult.done) {
            provLog("stream done — USB re-enumerated");
            throw new Error("stream-ended");
          }
          value = raceResult.value;
          if (value && value.length > 0) {
            const chunk = dec.decode(value);
            provLog(`← rx (${value.length}B): ${JSON.stringify(chunk)}`);
          } else {
            provLog(
              `← timeout tick (state=${state}, buf=${JSON.stringify(buf.slice(-80))})`,
            );
          }
        } catch (err) {
          provLog(`read error: ${err} — assuming mid-reboot disconnect`);
          try {
            reader.releaseLock();
          } catch {
            /* empty */
          }
          onStatus("Device rebooting, waiting...");
          try {
            await port.close();
          } catch {
            /* empty */
          }
          provLog("sleeping 3 s before reopen...");
          await sleep(3_000);
          provLog("attempting reopen...");
          await ensureOpen().catch((e) => provLog(`reopen failed: ${e}`));
          reader = port.readable!.getReader();
          activeReader = reader;
          provLog("reader re-acquired after reboot");
          buf = "";
          continue;
        }

        if (value && value.length > 0) buf += dec.decode(value);

        if (state === "ssid") {
          if (buf.includes("Enter WiFi SSID:")) {
            provLog("matched 'Enter WiFi SSID:' → sending SSID, moving to pwd");
            onStatus("Sending SSID...");
            await send(ssid);
            state = "pwd";
            buf = "";
          }
        } else if (state === "pwd") {
          if (buf.includes("Enter WiFi SSID:")) {
            provLog("saw SSID prompt while in pwd state — re-sending SSID");
            onStatus("Sending SSID...");
            await send(ssid);
            buf = "";
          } else if (buf.includes("Enter WiFi Password:")) {
            provLog(
              "matched 'Enter WiFi Password:' → sending password, moving to confirm",
            );
            onStatus("Sending password...");
            await send(password);
            state = "confirm";
            buf = "";
          }
        } else if (state === "confirm") {
          if (buf.includes("Save credentials?")) {
            provLog(
              "matched 'Save credentials?' → sending y, moving to result",
            );
            onStatus("Saving...");
            await send("y");
            state = "result";
            buf = "";
          }
        } else if (state === "result") {
          if (buf.includes("Credentials saved successfully!")) {
            provLog("matched 'Credentials saved successfully!' → done!");
            onStatus("WiFi configured!", true);
            return;
          }
          if (buf.includes("Error saving credentials")) {
            provLog(
              "matched 'Error saving credentials' → waiting for reboot + re-prompt",
            );
            onStatus("Rebooting to retry, please wait...");
            state = "ssid";
            buf = "";
          }
        }
      }
      provLog("timed out after 2 minutes");
      throw new Error("Configuration timed out after 2 minutes");
    } finally {
      try {
        reader.releaseLock();
      } catch {
        /* empty */
      }
      activeReader = null;
      provLog("reader released");
    }
  }

  return onActivate;
}

function sleep(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}
