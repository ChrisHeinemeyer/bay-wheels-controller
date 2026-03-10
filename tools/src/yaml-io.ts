import yaml from 'js-yaml';
import type { SavedSession, BoundingBox, StationMapping } from './types';

export function buildSession(
  mappings: StationMapping[],
  bbox: BoundingBox | null,
): SavedSession {
  return {
    created_at: new Date().toISOString(),
    bounding_box: bbox,
    mappings: [...mappings].sort((a, b) => a.bit_position - b.bit_position),
  };
}

export function sessionToYaml(session: SavedSession): string {
  return yaml.dump(session, { lineWidth: -1, sortKeys: false, quotingType: '"' });
}

export function downloadYaml(session: SavedSession): void {
  const content = sessionToYaml(session);
  const blob = new Blob([content], { type: 'text/yaml;charset=utf-8' });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = `board-mapping-${Date.now()}.yaml`;
  anchor.click();
  URL.revokeObjectURL(url);
}

export function promptLoadYaml(): Promise<SavedSession | null> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = '.yaml,.yml';
    input.addEventListener('change', () => {
      const file = input.files?.[0];
      if (!file) { resolve(null); return; }
      const reader = new FileReader();
      reader.onload = () => {
        try {
          const data = yaml.load(reader.result as string) as SavedSession;
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
