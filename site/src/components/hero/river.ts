// Every remote photo in assets/remotes joins the river.
const files = import.meta.glob<string>('../../assets/remotes/*.webp', { query: '?url', import: 'default', eager: true });
const remotes = Object.entries(files).sort(([a], [b]) => a.localeCompare(b)).map(([, url]) => url);

// Deterministic per-column noise, so the server render is stable between builds.
const noise = (i: number, salt: number) => {
  let h = Math.imul(i + 1, 0x9e3779b1) ^ Math.imul(salt + 7, 0x85ebca6b);
  h = Math.imul(h ^ (h >>> 15), 0x2c1b3c6d);
  return ((h ^ (h >>> 13)) >>> 0) / 4294967296;
};

export interface Column { remotes: string[]; style: string }

/** Each column cycles through every remote from its own starting point, so neighbours never line up. */
export function riverColumns(count: number): Column[] {
  return Array.from({ length: count }, (_, i) => {
    const start = (i * 2) % remotes.length;
    const order = [...remotes.slice(start), ...remotes.slice(0, start)];
    const speed = 70 + noise(i, 1) * 50;
    return {
      remotes: order,
      style: [
        `--speed:${speed.toFixed(1)}s`,
        `--delay:${(-noise(i, 2) * speed).toFixed(1)}s`,
        `--lag:${Math.round(-noise(i, 3) * 200)}px`,
      ].join(';'),
    };
  });
}
