/**
 * Extract firmware version from an ESP32 app binary.
 *
 * The ESP-IDF app descriptor (esp_app_desc_t) is at the start of the DROM segment.
 * We search for the magic word 0xABCD5432; the version string is 32 bytes at offset +16.
 * See: https://docs.espressif.com/projects/esp-idf/en/latest/esp32s3/api-reference/system/app_image_format.html
 */
const ESP_APP_DESC_MAGIC = new Uint8Array([0x32, 0x54, 0xcd, 0xab]); // 0xABCD5432 LE
const VERSION_OFFSET = 16;
const VERSION_LEN = 32;

export function extractFirmwareVersion(data: Uint8Array): string | null {
  if (data.length < ESP_APP_DESC_MAGIC.length + VERSION_OFFSET + VERSION_LEN) {
    return null;
  }
  const minChunk = ESP_APP_DESC_MAGIC.length + VERSION_OFFSET + VERSION_LEN;
  for (let i = 0; i <= data.length - minChunk; i++) {
    let match = true;
    for (let j = 0; j < ESP_APP_DESC_MAGIC.length; j++) {
      if (data[i + j] !== ESP_APP_DESC_MAGIC[j]) {
        match = false;
        break;
      }
    }
    if (match) {
      const versionStart = i + VERSION_OFFSET;
      const versionBytes = data.slice(versionStart, versionStart + VERSION_LEN);
      const nullIdx = versionBytes.indexOf(0);
      const len = nullIdx >= 0 ? nullIdx : VERSION_LEN;
      const str = new TextDecoder().decode(versionBytes.slice(0, len));
      return str.trim() || null;
    }
  }
  return null;
}
