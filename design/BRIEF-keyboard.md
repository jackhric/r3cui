# Keyboard brief — entering a Wi-Fi password with a d-pad

Read `BRIEF.md` for the device (480×800 portrait screen, physical d-pad with centre OK, BACK key)
and `round3/quiet-a.html` for the visual language you must match. This file is the contract.

## The job

One screen: the user picked the network **`Guest`** from a list and must type its password.
The primary input is the **physical d-pad** (up, down, left, right, OK) plus the BACK key. Touch
also works, but design for the d-pad first: every key and action must be reachable and operable
with those six inputs alone, and the focused key must be unmistakable from a couch in the dark.

Typing a 12-character mixed password should take as few presses as you can manage. Think about
wrap-around at the edges, where focus lands after a mode change, and how to reach Delete and Join
quickly.

## Must have

- Header: back chevron + `Guest` (same pattern as the menu pages in quiet-a).
- A password field showing what has been typed, in clear text (this is a living-room remote),
  with a caret. Start it pre-filled with `Bl4de` so the screen is not empty at rest.
- Lowercase, uppercase, digits and common symbols (at least `! @ # $ % & * - _ . ? /`).
- Delete, Space, and **Join** (the commit action). Join is dim until 8+ characters are typed.
- BACK key: deletes the last character; when the field is empty it would leave the screen (just
  flash the back chevron).
- OK on Join with 8+ characters: show a brief `Joining…` state in place of the keyboard, then
  return to the keyboard after ~1.5 s (it is a mock).

## Visual language (Quiet)

- Pure `#000` background. Font: Hanken Grotesk (Google Fonts), weights 200–600, tabular numerals.
- Text `#F1F3F3`, secondary `#8B9195`, tertiary `#53595D`. Hairlines `rgba(255,255,255,.08)`.
- One accent, red `#E8343A`, used only for focus and the live thing. Keys may have barely-there
  fills (`#0E0E0F`–`#161617`); no outlines on every key, no gradients, no shadows, no blur.
- Thick, rounded shapes where there are shapes (the product's bars are 14px tall, fully rounded).
- Nothing under 18px except small mode labels (13px+, letter-spaced). Touch targets ≥ 44px for
  keys is acceptable here because the d-pad is primary; the action keys should be ≥ 56px.
- Must be drawable in a 2D toolkit later: flat fills, rounded rects, text, opacity, simple moves.

## Fragment contract (the page depends on it)

Write exactly one file, `design/keyboard/<slug>.html`, containing in this order:

```html
<style> /* every selector starts with .kb-<slug>; keyframes named <slug>-... */ </style>
<article class="screen kb-<slug>" data-name="<Name>" data-note="<one plain sentence>">
  ... the whole 480×800 screen ...
</article>
<script>
(() => {
  const root = document.currentScript.previousElementSibling;   // the <article>
  // ... build keys, keep state ...
  (window.KB = window.KB || {})['<slug>'] = {
    key(name) { /* name is one of: 'up' 'down' 'left' 'right' 'ok' 'back' */ },
  };
})();
</script>
```

- The page already styles `.screen` as `width:480px; height:800px; position:relative;
  overflow:hidden; background:#000; box-sizing:border-box`, loads Hanken Grotesk, and calls
  `window.KB['<slug>'].key(...)` from its own on-page d-pad and from the arrow/Enter/Backspace
  keys. **Do not add your own `keydown` listeners.** Do handle `click` on your keys for touch.
- No `id` attributes, no globals other than `window.KB[...]`, no external resources, no libraries.
  Query only inside `root`. Use `textContent`, never `innerHTML` with typed text.
- Set `font-family` on `.kb-<slug>` itself. Do not style `body`, `*` or bare elements.
- Everything must fit inside 480×800: sum your vertical layout.
