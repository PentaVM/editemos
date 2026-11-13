---
name: frame-review
description: Checking thousands of frames without watching them all. Stills at start, middle and end of each shot, a script that flags layers touching each other or landing on interface text, contact sheets every half second with the time on each frame, a legibility test for screens, and close looks only where the script points. Read before showing any finished cut.
---

# Frame-by-frame review

Nobody can look at 10,000 frames. Combine three things.

## 1. A script that checks every frame

For each frame, compute:

- the alpha bounding box of each overlay;
- the subtitles drawn by libass over black (their real footprint);
- the edges (text and interface) of the background image.

It warns when two layers touch, or when a layer lands on text or interface of the picture
outside a dark scrim. Read the layers from the cut table, render each one alone, and compare
boxes; it does not need to understand the picture.

## 2. Contact sheets every 0.5 s

A frame every half second, the time written on each frame, 60 frames (30 s) per sheet. The
whole video on a few pages: overlaps, a title that stayed too long and a missing shot are
visible at a glance.

```bash
ffmpeg -i cut.mp4 -vf "fps=2,scale=480:-1,drawtext=text='%{pts\:hms}':x=8:y=8:fontcolor=white:box=1:boxcolor=black@0.6,tile=6x10" sheet-%02d.png
```

## 3. Look closely only where it matters

Zoom on the frames the script flags, and on the start, middle and end of every shot.

## What this found in one 2:48 demo

- subtitles on top of the recorded interface's chips;
- titles that carried over into the next shot;
- metrics on top of a globe;
- subtitles over a painting and over an API-call card;
- a title brushing labels during a zoom;
- a dotted line crossing a subtitle (it read as strikethrough).

## Stills before the expensive render

For 3D scenes and long renders, render control stills first: 1280x720, 16 samples, at the
start, middle and end of each scene, on one contact sheet. Check objects covering screens,
flicker and overlapping planes, text and logos zoomed in, background density and horizon, and
moving objects crossing each other or text (at the start, middle and end of each move).

## Legibility test for screens

One frame per scene that contains a screen, at final resolution, with the final glare and
colour transform. Screens look small and pass through the view transform (AgX, Filmic): if the
interface cannot be read there, fix it before launching the batch.

## In editemos

- `editemos_motion render` returns `review`: one still per settle point with layout findings
  (text off frame, outside the safe margin, overlapping, too small, low contrast).
- `editemos_clips action=look from to` returns a grid of frames with their times.
- `editemos_clips action=build` returns a review sheet per clip.
- `editemos_deliver` checks the master with ffprobe (size, frame rate, duration, audio).
