// Every clip in assets/clips joins the river: <name>.webm + <name>.mp4 + <name>.webp (poster).
const files = import.meta.glob<string>('../../assets/clips/*.{webm,mp4,webp}', { query: '?url', import: 'default', eager: true });
const byName = new Map<string, Record<string, string>>();
for (const [path, url] of Object.entries(files)) {
  const m = /([^/]+)\.(webm|mp4|webp)$/.exec(path)!;
  byName.set(m[1], { ...byName.get(m[1]), [m[2]]: url });
}
export interface Clip { webm: string; mp4: string; poster: string }
const clips: Clip[] = [...byName]
  .filter(([, f]) => f.webm && f.mp4 && f.webp)
  .sort(([a], [b]) => a.localeCompare(b))
  .map(([, f]) => ({ webm: f.webm, mp4: f.mp4, poster: f.webp }));

export interface Column { clips: Clip[]; style: string }

const GOLDEN = 0.618034;

/**
 * Each column cycles through every clip from its own starting point. All columns drift at one speed,
 * so their offsets never change, and those offsets step by the golden ratio of a tile's height: no two
 * neighbours ever line up, and no pattern repeats across the river.
 */
export function riverColumns(count: number): Column[] {
  return Array.from({ length: count }, (_, i) => {
    const start = (i * 2) % clips.length;
    const order = [...clips.slice(start), ...clips.slice(0, start)];
    // As a fraction of one tile plus its gap; the CSS turns it into pixels for the tile size in use.
    return { clips: order, style: `--stagger:${((i * GOLDEN) % 1).toFixed(3)}` };
  });
}
