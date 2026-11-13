---
name: screen-capture
description: Recording a real web or desktop app for a video. Deterministic demo mode, frame-by-frame capture at exact 60 fps, measuring how often the screen really updates, virtual displays, consent before capturing a real desktop, encoder tests, evidence per take, and smoothing slow captures with mpdecimate and minterpolate. Read before recording any interface.
---

# Recording an app for a video

The interface on screen must be the real one, and it must move smoothly. Both are harder than
they look.

## Prepare the app

- **Deterministic demo mode.** Replay recorded data (`recording.json`) through the whole path,
  without errors. Never record against a live API: it is slow, it changes, it fails on the day.
- **Record exactly the code you present.** Build that commit in its own folder and keep an
  archive of the source next to the captures.
- **Show the real state.** Background services up, so the app says "live" and not "not
  responding". Theme and language fixed by argument (`--theme dark`, `APP_UI_LANG=en`), not by
  whatever was saved last.
- **Readable on video.** Heavier text weight, darker greys, no scrollbar, no cursor unless the
  cursor is the point. A redesign that was delivered replaces the old one; it is not merged.
  Check narrow widths (320, 360, 375 px) for web apps.

## Web apps: frame by frame

Drive the page with Playwright (or the browser's DevTools protocol) and capture **frame by
frame at exactly 60 fps**, on the scene's own clock: advance virtual time one frame, take the
frame. The key action lands on the frame the script asks for. Real-time capture drops frames
and drifts.

## Desktop apps

1. **Measure before recording how often the screen really changes.** Record 6 s and count the
   frames that differ from the previous one. Measured on one machine (RTX 3070, Qt app in WSL):
   - Xvfb with software rendering: ~11 updates/s;
   - Xvfb with the GPU (d3d12): ~9/s, slower;
   - WSLg on the real display: ~8/s reach Windows, because they pass through RDP.

   If it does not reach 60, plan the smoothing below. If the app can be stepped frame by frame
   (like a page under Playwright), prefer that.
2. **Try tricks for 10 seconds before betting the plan on them.** `QSG_FIXED_ANIMATION_STEP`
   did nothing: the app only draws when something changes.
3. **A virtual display (Xvfb) whenever possible.** It touches nobody's desktop. Linux:
   `ffmpeg -f x11grab -framerate 60 -video_size 1920x1080 -i :99`.
4. **If the real screen must be recorded** (Windows: `ffmpeg -f lavfi -i ddagrab=framerate=60`
   with NVENC; or OBS Studio):
   - ask the owner explicitly, turn on Do Not Disturb, and nobody touches the PC: personal
     notifications end up in the capture;
   - never capture the full screen without that agreement;
   - a "human mouse" can be simulated with `SetCursorPos` from PowerShell.
5. **Test the encoder with 2 s before the real take.** A newer ffmpeg wanted NVENC 13.1 while
   the driver had 13.0; an older build that shipped with Kdenlive worked.
6. **Between WSL and Windows, orchestrate from the side that records.** Launching Windows
   `.exe` from WSL fails intermittently; PowerShell records and calls `wsl` for the Linux part.
7. **Every real take leaves evidence:** a `.evidence.txt` with the code used, the machine, and
   the time of each action with its receipt.
8. **Keep earlier takes in subfolders** (`_v1_…`, `_v2_…`); never overwrite.
9. **A screen that appears in another scene gets its own video**, with a suffix:
   `CODE__S04.mp4`.

## Smoothing a slow capture to 60 fps

```bash
ffmpeg -i take.mp4 -vf "mpdecimate=max=8,setpts=N/FRAME_RATE/TB,minterpolate=fps=60:mi_mode=mci:mc_mode=aobmc:me_mode=bidir:vsbmc=1" -c:v libx264 -crf 14 take60.mp4
```

- Drop repeated frames with `mpdecimate` **and `max=8`**: without it a static stretch collapses
  to one frame and the clip comes out empty or short.
- Fill with `minterpolate` (motion-compensated). Only on takes that move; a nearly still
  globe does not need it.
- Zoom into the invented frames. Text and nodes hold up; a sudden pointer jump leaves a faint
  double cursor for one frame.
- **Parallelise by memory, not by cores.** `minterpolate` uses one core and ~1.3 GB per process
  at 1080p (~75 s of CPU per second of video). Ten in parallel exhausted 8.9 GB of RAM; five
  worked. Split into 2.5 s pieces with 0.5 s of margin on each side.

## On a 3D screen

When the recording goes inside a rendered screen (see `3d-scenes`), validate each quad before
pasting, read videos one frame at a time through a pipe, and if a take shows something half
loaded at that moment, use another stretch or another take for that screen.

## Tools

OBS Studio for real-time capture when the app cannot be stepped; Playwright for web apps;
ffmpeg (`x11grab`, `ddagrab`, `gdigrab`, `avfoundation`) for scripted capture.
