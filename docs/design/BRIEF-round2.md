# Round 2 brief — refining "Signal"

Read `BRIEF.md` first: the device, the product, the hard requirements and the **fragment
contract** all still apply. This file overrides it where they differ.

## What the owner said about round 1

- **Signal is the favourite** (`round1/signal.html`): cyan-on-black, the meter as the instrument.
- But **dial the retro back**. No fluorescent-display pastiche, no chunky VFD segments, no
  monospace captions everywhere, no outlined "modules". It should be **modern, in the manner of
  Monolith** (`round1/monolith.html`): typographic, calm, hairlines, big confident numerals.
- **Don't overwhelm with useless information.** Round 1 showed too much.
- **Use real logos** wherever a logo can stand for a format or source (Dolby, DTS, PlayStation…).
- **The menu must be very simple to use.**

Read both round-1 files before designing so you know what is being merged.

## Information diet (applies to every screen)

Each screen answers one question at a glance. Cut everything else.

- Never show: dB conversions, dBm, IP addresses, firmware numbers, entity names, technical
  captions (`INPUT`, `MODE`, `AUDIO` labels on obvious things), hard-key legends, tick-label
  clutter, redundant units.
- Status line: clock (`21:14`) and a battery glyph only. No WiFi icon unless disconnected.
  One quiet menu affordance (a small glyph, top right) on the non-menu screens.
- At most **two** touch actions on the Kodi screens (Subtitles, Audio). None on Receiver beyond
  opening the input list.
- Small text exists only where it earns its place. If in doubt, remove it.

## Logos

The page provides an SVG sprite. Use it like this (the glyph takes `currentColor` via `fill`):

```html
<svg class="logo" viewBox="0 0 24 24" width="28" height="28" fill="currentColor"><use href="#logo-dolby"/></svg>
```

Available ids: `logo-dolby` (the double-D), `logo-dts`, `logo-kodi`, `logo-playstation`,
`logo-playstation5` (wide wordmark-style PS5 mark), `logo-xbox`, `logo-android`,
`logo-bluetooth`, `logo-denon`, `logo-homeassistant`.

- Audio format = logo + at most two short words. Receiver screen: Dolby mark + `Digital`.
  Kodi playing: DTS mark + `HD MA 7.1`. Never the long codec string.
- Inputs get their logo: PlayStation 5, Xbox 360, Kodi, Android TV, Bluetooth. `HEOS Music` and
  `NET` have no sprite: draw a simple solid glyph (music note, globe) at the same visual weight.
- Logos are white or grey, never tinted with the accent, and never recoloured per brand.

## The seven screens (exactly these, in this order)

| `data-screen` | `data-label` | What it shows |
|---|---|---|
| `receiver` | Receiver | Input is `PLAYSTATION 5`. Volume `62` is the hero with the meter. Input shown as logo + name; tapping it opens the list. Sound format as Dolby logo + `Digital`. Nothing else. |
| `inputs` | Inputs | The seven inputs as large rows with logos, current one marked. No sound modes. |
| `kodi-playing` | Kodi · playing | *Blade Runner 2049*. Progress is the hero: elapsed `1:12:08`, `1:31 left`, `ends 22:47` (pick the two most useful to make large; the third may be small or dropped). Artwork stand-in (abstract teal-and-orange dusk, CSS/SVG only). DTS logo + `HD MA 7.1`. Two actions: Subtitles, Audio. Volume `62` small. |
| `kodi-browse` | Kodi · browsing | Nothing playing. Almost empty: Kodi logo, volume `62`, and that is nearly all. This screen should feel like the remote getting out of the way. |
| `volume` | Volume | Overlay while a hard volume key is held: `64`, the meter, nothing else. Show the muted state as a second small example only if it fits cleanly; otherwise omit. |
| `menu` | Menu | Top level, five rows only: **Wi-Fi** (`HomeNetwork`), **Display** (`90%`), **Sound & haptics** (`On`), **Power**, **About**. Each row at least 96px tall, one value at the right, a chevron. One row shown focused, because the d-pad can drive this list too. |
| `menu-display` | Menu · Display | One detail page to prove the pattern: a back affordance and title, a brightness slider at `90%`, an `Auto brightness` switch (on), and `Screen off after` → `60 s`. Three controls, no more. |

## Shared character (all round-2 variants)

- Pure black. One accent: Signal's cyan-teal (around `#63E6D9`; you may tune it slightly). The
  accent marks only the live thing on a screen. Everything else is white and one or two greys.
- The meter survives as *the* signature, redrawn in a modern way (your variant says how).
- No cards, no outlined boxes, no monospace body text. Hairlines and space do the grouping.
- All seven screens share one grid, one status line, one icon weight, one type scale.
