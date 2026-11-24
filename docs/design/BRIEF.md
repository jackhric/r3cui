# Prototype brief — single-purpose home theater remote

HTML mockups of the on-device UI for an Unfolded Circle Remote 3 running a custom frontend.
Each variant is one HTML fragment in `docs/design/variants/<slug>.html`; `docs/design/build.sh` stitches
them into one flick-through page. The real UI will later be rebuilt in Slint, so keep to things
a 2D GLES2 toolkit can draw: flat fills, gradients, rounded rects, strokes, arcs, text, opacity.
No backdrop-filter blur, no CSS 3D.

## The device

- Screen: 480×800 portrait touchscreen, about 3.2" (very dense, held at arm's length in a dark
  theater). Below it the remote has physical keys: BACK, HOME, d-pad with centre OK, VOLUME +/−,
  MUTE, CHANNEL +/−, PREV, PLAY/PAUSE, NEXT, MENU, STOP, RECORD, four colour keys, and a
  capacitive touch slider. Every hard-key press gives a haptic tick.
- Because navigation lives on the hard keys, **the screen never shows a d-pad or transport
  buttons**. The screen is for status, progress and the few actions that have no hard key.

## What the remote does (the whole product)

1. **Receiver** (Denon AVR-S760H, via Home Assistant): volume, mute, input, sound mode.
   Hard VOLUME/MUTE keys always drive the receiver, on every screen.
2. **Kodi**: when the receiver input is `KODI`, the remote becomes a Kodi remote. Hard keys
   navigate; the screen shows what is playing and how far along it is.
3. **Settings**: WiFi, battery, display, power, as exposed by the remote's core.

## Hard requirements

- Screen background is pure `#000`. Surfaces, if any, are barely-there (≤ `#141414`) or just
  hairline outlines. No grey page backgrounds, no light theme.
- Premium home theater feel: calm, confident, lots of black, one idea per screen. Not a
  dashboard, not a phone app.
- Type: nothing meaningful under 18px; small uppercase labels may go to 13px with letter-spacing.
  Primary readouts are large (the volume number can be 150px+). Use `font-variant-numeric:
  tabular-nums` on changing numbers.
- Touch targets at least 56px tall.
- Use the real data below. No lorem ipsum, no invented features.

## The six screens (exactly these, in this order)

| `data-screen` | `data-label` | What it shows |
|---|---|---|
| `receiver` | Receiver | Home when the input is not Kodi (use `PLAYSTATION 5`). Volume `62`, sound mode `DTS SURROUND`, current input, way to open the input list, mute and power affordances. |
| `inputs` | Inputs | Input picker: `ANDROID TV`, `Bluetooth`, `HEOS Music`, `KODI`, `NET`, `PLAYSTATION 5`, `XBOX 360`. `PLAYSTATION 5` is current. May also offer sound modes `MOVIE`, `MUSIC`, `GAME`, `STEREO`, `PURE DIRECT`. |
| `kodi-playing` | Kodi · playing | Now playing: *Blade Runner 2049* (2017), 2 h 44 min long, at 1:12:08, 1:31:52 left, ends 22:47. Audio `DTS-HD MA 7.1 · English`, subtitles `English`. A clear progress indicator is the centrepiece. Touch actions with no hard key: Subtitles, Audio, Info, Context menu. Small receiver volume readout (`62`). Artwork exists on the real device (a poster URL); stand in for it with an abstract CSS/SVG composition in teal-and-orange dusk tones, never an external image. |
| `kodi-browse` | Kodi · browsing | Nothing playing; the user is navigating Kodi's menus on the TV with the hard keys. Calm screen. Show the input (`KODI`), volume (`62`), and a short legend of what the less obvious hard keys do here (BACK = back, HOME = Kodi home, MENU = context menu, CH +/− = page up/down). A few touch shortcuts: Home, Videos, Info, Fullscreen. |
| `volume` | Volume overlay | The transient overlay shown for ~2 s when a hard volume key is pressed, drawn over the dimmed `kodi-playing` screen. Volume `64` going up from `62`. Also show how mute would look, somewhere secondary. |
| `settings` | Settings | WiFi `HomeNetwork` (−68 dBm, 192.168.1.50); Battery 50 % charging; Display brightness 90 % (auto); Key backlight 50 %; Screen off after 60 s; Standby after 3 min; Haptics on; Sounds 80 %; Power actions: Standby, Restart, Power off; About: Remote 3, firmware 2.8.4. |

Every screen except `volume` carries a slim status line: 24-hour clock (`21:14`), WiFi, battery.

## Fragment contract (must follow exactly, the build script depends on it)

```html
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=...&display=swap">
<style>
  /* every selector starts with .v-<slug>; keyframe names start with <slug>- */
</style>
<section class="variant v-<slug>" data-name="<Name>" data-fonts="<Font A + Font B>"
         data-note="<one plain sentence describing the direction>">
  <article class="screen" data-screen="receiver" data-label="Receiver"> ... </article>
  <article class="screen" data-screen="inputs" data-label="Inputs"> ... </article>
  <article class="screen" data-screen="kodi-playing" data-label="Kodi · playing"> ... </article>
  <article class="screen" data-screen="kodi-browse" data-label="Kodi · browsing"> ... </article>
  <article class="screen" data-screen="volume" data-label="Volume overlay"> ... </article>
  <article class="screen" data-screen="settings" data-label="Settings"> ... </article>
</section>
```

- The shell already gives `.screen` `width:480px; height:800px; position:relative;
  overflow:hidden; background:#000; box-sizing:border-box`. Design inside that box in px.
  Content must not overflow it: check your vertical sums.
- No `<html>`, `<head>`, `<body>`, no `<script>`, no `id` attributes except SVG defs, whose ids
  must start with `<slug>-`. No external images or icon fonts: inline SVG icons only, drawn
  consistently (one stroke weight per variant).
- Fonts only from Google Fonts, with a real fallback stack. Set `font-family` on
  `.v-<slug> .screen`, never globally. Do not style `body`, `*`, or bare element selectors.
- Subtle CSS animation is welcome where it shows intent (a progress bar creeping, a volume
  number settling) and must respect `prefers-reduced-motion`. Every screen must look complete
  as a still frame.
- All six screens must feel like one product: same grid, status line, icon set and type scale.
