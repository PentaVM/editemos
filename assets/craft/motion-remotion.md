---
name: motion-remotion
description: Writing editemos motion scenes the Remotion way (interpolate, spring, Sequence, Series, Easing) without React, with sound tied to the motion, springs that settle, and the review loop that checks every settle point. Read before writing a scene by hand.
---

# Motion scenes, the Remotion way

An editemos scene is an HTML page. Every value on screen is a pure function of time, so a
frame can be rendered on its own, in any order, identically every time. Remotion works the
same way; its functions exist here with the same names and the same maths, minus React.

## Skeleton

```html
<div id="card">One clear view</div>
<script>
const { interpolate, spring, Easing, Sequence, Series, Sfx, useVideoConfig } = Remotion;
const card = document.getElementById('card');
Motion.composition({ id: 'reveal', durationInFrames: 90, fps: 30, render(frame) {
  const { fps } = useVideoConfig();
  const enter = spring({ frame, fps, config: { damping: 14, stiffness: 120 } });
  card.style.transform = `translateY(${interpolate(enter, [0, 1], [60, 0])}px) scale(${0.94 + 0.06 * enter})`;
  Sequence({ from: 40, durationInFrames: 50 }, (f) => {
    card.style.clipPath = `inset(0 ${interpolate(f, [0, 12], [100, 0], { easing: Easing.bezier(0.16, 1, 0.3, 1), extrapolateRight: 'clamp' })}% 0 0)`;
    Sfx({ kind: 'whoosh' });           // at the start of this Sequence
  });
} });
</script>
```

- `render(frame)` writes the DOM (or a canvas) for that frame. No timers, no
  `requestAnimationFrame`, no `Math.random()` (use `random(seed)`).
- `Sequence({ from, durationInFrames, el })`: children see a frame that starts at 0; pass `el`
  to hide that element outside its range. `Series([...])` puts sequences back to back.
- `spring` uses the damped-oscillator closed form; values agree with Remotion's. Use
  `durationInFrames` to make a spring settle on a given frame, `delay` to start it later.
- The timeline API (`Motion.timeline().fromTo(...)`) still works and is shorter for simple
  entrances; mix freely.

## Sound

Sound belongs to the motion that causes it.

- On a tween: `tl.fromTo('.title', {...}, {...}, { at: 0.2, dur: 0.5, ease: 'slam', sfx: { kind: 'thud', at: 'end' } })`.
- In a composition: `Sfx({ kind: 'pop', from: 5 })` inside a `Sequence`.
- Anywhere: `Motion.sfx(seconds, kind, { gain, pitch, pan, dur })`; `Motion.beat(n, bpm)` gives
  the time of beat n on a music grid.
- Kinds: click, tick, pop, whoosh, thud, boom, riser (ends on its time), hit, type, glitch,
  shimmer, or `file` with `src` (an audio file next to the scene, `Remotion.Audio({ src, from })`).

The renderer synthesises it, sample-exact, and puts it in the video (PCM in .mov, Opus in
.webm, `audio.wav` beside a PNG sequence).

## What reads as cheap

- One easing for everything, or linear `interpolate` on frames (the linter counts these as
  stock easing). Use springs and named curves; vary durations (slowest about 3x the fastest).
- Fade plus a small slide on every element. Use masks (`clip-path`), scale, rotation, cuts on
  motion.
- Everything starting at frame 0. Offset the first move 3 to 9 frames.
- Text centred, huge, with a soft drop shadow.
- Bounce and elastic easings.

## Review loop (do it, every time)

`editemos_motion render` returns `review`: one still per moment the scene comes to rest (a
couple of frames after each move ends), and checks the layout there: text cut by the frame
edge, outside the 5% safe margin, overlapping text, text too small for a phone, low contrast
against its own background. Then:

1. Look at the review stills over real footage (`over=<frame.png>`).
2. Is every word readable? Did each element land before the next appeared? Does anything
   collide or touch an edge? Does the sound hit where the motion lands?
3. Fix the scene file (not a new script) and render again. Two rounds at most, then show the
   user and say what changed.
