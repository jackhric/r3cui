// The portfolio's wordmark lens (jackhric/portfolio, wordmark-lens.ts), for an SVG logo instead of
// text: the pointer pulls and swirls the logo's pixels around it, easing back when it leaves.
const PULL = 0.28;
const SWIRL = 0.35;
const RADIUS = 0.9;
const PAD = 0.15;

function cssColorRgb(color: string): [number, number, number] {
  const c = document.createElement('canvas').getContext('2d', { willReadFrequently: true })!;
  c.canvas.width = c.canvas.height = 1;
  c.fillStyle = color;
  c.fillRect(0, 0, 1, 1);
  const [r, g, b] = c.getImageData(0, 0, 1, 1).data;
  return [r, g, b];
}

export function mountLogoLens(root: HTMLElement) {
  const holder = root.querySelector<HTMLElement>('[data-lens-art]');
  const svg = holder?.querySelector('svg');
  const canvas = root.querySelector<HTMLCanvasElement>('canvas');
  const ctx = canvas?.getContext('2d');
  if (!holder || !svg || !canvas || !ctx) return;
  if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;

  let W = 0, H = 0, dpr = 1, radius = 1;
  let mask = new Float32Array(0), image: ImageData | null = null, ink: [number, number, number] = [255, 255, 255];
  let raf = 0, last = 0, amount = 0, hovering = false;
  let target: [number, number] = [0, 0], hole: [number, number] = [0, 0];
  let building = 0;

  async function build() {
    const rect = svg!.getBoundingClientRect();
    if (!rect.width) return;
    const ticket = ++building;
    dpr = Math.min(2, devicePixelRatio || 1);
    // The logo's height stands in for the wordmark's font size.
    radius = rect.height * RADIUS * dpr;
    const pad = Math.ceil(rect.height * PAD);
    const cw = Math.ceil(rect.width) + pad * 2, ch = Math.ceil(rect.height) + pad * 2;
    // Only the shape's alpha is needed, so it is drawn in any solid colour.
    const art = new Image();
    art.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(new XMLSerializer().serializeToString(svg!).replaceAll('currentColor', '#000'))}`;
    await art.decode().catch(() => {});
    if (ticket !== building) return;
    W = Math.round(cw * dpr);
    H = Math.round(ch * dpr);
    canvas!.width = W;
    canvas!.height = H;
    canvas!.style.width = `${cw}px`;
    canvas!.style.height = `${ch}px`;
    canvas!.style.left = canvas!.style.top = `${-pad}px`;
    const off = document.createElement('canvas').getContext('2d', { willReadFrequently: true })!;
    off.canvas.width = W;
    off.canvas.height = H;
    off.drawImage(art, pad * dpr, pad * dpr, rect.width * dpr, rect.height * dpr);
    const alpha = off.getImageData(0, 0, W, H).data;
    mask = new Float32Array(W * H);
    for (let i = 0; i < mask.length; i++) mask[i] = alpha[i * 4 + 3] / 255;
    image = ctx!.createImageData(W, H);
    ink = cssColorRgb(getComputedStyle(svg!).color);
    holder!.dataset.lensReady = '';
    draw();
  }

  function at(x: number, y: number) {
    if (x < 0 || y < 0 || x > W - 1 || y > H - 1) return 0;
    const x0 = Math.floor(x), y0 = Math.floor(y), x1 = Math.min(W - 1, x0 + 1), y1 = Math.min(H - 1, y0 + 1);
    const fx = x - x0, fy = y - y0;
    const top = mask[y0 * W + x0] + (mask[y0 * W + x1] - mask[y0 * W + x0]) * fx;
    const bot = mask[y1 * W + x0] + (mask[y1 * W + x1] - mask[y1 * W + x0]) * fx;
    return top + (bot - top) * fy;
  }

  function draw() {
    if (!image) return;
    const d = image.data;
    const [ir, ig, ib] = ink;
    const [hx, hy] = hole;
    const r2 = radius * radius;
    for (let y = 0; y < H; y++)
      for (let x = 0; x < W; x++) {
        const i = y * W + x, o = i * 4;
        let v = mask[i];
        if (amount > 0.001) {
          const dx = x - hx, dy = y - hy;
          const fall = amount / (1 + (dx * dx + dy * dy) / r2);
          const scale = 1 + PULL * fall, turn = SWIRL * fall * fall, cos = Math.cos(turn), sin = Math.sin(turn);
          v = at(hx + (dx * cos - dy * sin) * scale, hy + (dx * sin + dy * cos) * scale);
        }
        d[o] = ir;
        d[o + 1] = ig;
        d[o + 2] = ib;
        d[o + 3] = v * 255;
      }
    ctx!.putImageData(image, 0, 0);
  }

  function tick(now: number) {
    const step = Math.min(4, (now - (last || now)) / 16.7);
    last = now;
    const ease = 1 - Math.pow(0.88, step);
    amount += ((hovering ? 1 : 0) - amount) * ease;
    hole = [hole[0] + (target[0] - hole[0]) * ease * 1.6, hole[1] + (target[1] - hole[1]) * ease * 1.6];
    if (!hovering && amount < 0.002) amount = 0;
    draw();
    raf = amount > 0 || hovering ? requestAnimationFrame(tick) : 0;
    if (!raf) last = 0;
  }

  function onMove(e: PointerEvent) {
    if (!image) return;
    const box = canvas!.getBoundingClientRect();
    target = [(e.clientX - box.left) * dpr, (e.clientY - box.top) * dpr];
    if (!hovering) {
      hovering = true;
      if (amount === 0) hole = target;
    }
    if (!raf) raf = requestAnimationFrame(tick);
  }

  new ResizeObserver(() => build()).observe(holder);
  root.addEventListener('pointermove', onMove);
  root.addEventListener('pointerleave', () => (hovering = false));
}
