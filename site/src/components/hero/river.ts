// Every clip in assets/clips joins the river: <name>.webm + <name>.mp4 + <name>.webp (poster).
const files = import.meta.glob<string>('../../assets/clips/*.{webm,mp4,webp}', { query: '?url', import: 'default', eager: true });
const byName = new Map<string, Record<string, string>>();
for (const [path, url] of Object.entries(files)) {
  const m = /([^/]+)\.(webm|mp4|webp)$/.exec(path)!;
  byName.set(m[1], { ...byName.get(m[1]), [m[2]]: url });
}
export interface Clip { webm: string; mp4: string; poster: string }
const complete = (f: Record<string, string> | undefined): Clip | undefined =>
  f?.webm && f.mp4 && f.webp ? { webm: f.webm, mp4: f.mp4, poster: f.webp } : undefined;
// Each recording comes in the regular light aluminium and as `<name>-dark`, the dark grey finish.
const clips = [...byName.keys()]
  .filter(name => !name.endsWith('-dark'))
  .sort()
  .flatMap(name => {
    const light = complete(byName.get(name));
    return light ? [{ light, dark: complete(byName.get(`${name}-dark`)) ?? light }] : [];
  });

// Deterministic noise, so every build picks the same finishes.
const noise = (i: number, salt: number) => {
  let h = Math.imul(i + 1, 0x9e3779b1) ^ Math.imul(salt + 7, 0x85ebca6b);
  h = Math.imul(h ^ (h >>> 15), 0x2c1b3c6d);
  return ((h ^ (h >>> 13)) >>> 0) / 4294967296;
};

export interface Column { clips: Clip[]; style: string }

const GOLDEN = 0.618034;

/**
 * Each column cycles through every clip from its own starting point, each in a light or dark finish. All columns drift at one speed,
 * so their offsets never change, and those offsets step by the golden ratio of a tile's height: no two
 * neighbours ever line up, and no pattern repeats across the river.
 */
export function riverColumns(count: number): Column[] {
  return Array.from({ length: count }, (_, i) => {
    const start = (i * 2) % clips.length;
    // About half the remotes in each column are the dark grey finish.
    const order = [...clips.slice(start), ...clips.slice(0, start)].map((c, k) => (noise(i * 31 + k, 10) < 0.5 ? c.dark : c.light));
    // As a fraction of one tile plus its gap; the CSS turns it into pixels for the tile size in use.
    return { clips: order, style: `--stagger:${((i * GOLDEN) % 1).toFixed(3)}` };
  });
}
