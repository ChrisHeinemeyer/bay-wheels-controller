import yaml from "js-yaml";
export function buildSession(mappings, bbox) {
  return {
    created_at: new Date().toISOString(),
    bounding_box: bbox,
    mappings: [...mappings].sort(
      (a, b) => a.row - b.row || a.column - b.column,
    ),
  };
}
export function sessionToYaml(session) {
  return yaml.dump(session, {
    lineWidth: -1,
    sortKeys: false,
    quotingType: '"',
  });
}
export function downloadYaml(session) {
  const content = sessionToYaml(session);
  const blob = new Blob([content], { type: "text/yaml;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = `board-mapping-${Date.now()}.yaml`;
  anchor.click();
  URL.revokeObjectURL(url);
}
export function promptLoadYaml() {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".yaml,.yml";
    input.addEventListener("change", () => {
      const file = input.files?.[0];
      if (!file) {
        resolve(null);
        return;
      }
      const reader = new FileReader();
      reader.onload = () => {
        try {
          const data = yaml.load(reader.result);
          resolve(data);
        } catch (e) {
          alert(`Failed to parse YAML: ${e}`);
          resolve(null);
        }
      };
      reader.readAsText(file);
    });
    input.click();
  });
}
