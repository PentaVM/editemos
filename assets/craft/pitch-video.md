---
name: pitch-video
description: End to end for a pitch, demo or launch video with narration, real screen recordings, graphics and optional 3D scenes. One cut table, script with text and pronunciation, a frame designed before any graphic, layers that never cross a cut, and a rebuild script that works without you. Read before planning any narrated product, hackathon or launch video.
---

# Pitch and demo videos

Brief, prototype, animate, render, ship. A pitch video is short (1-3 minutes) and dense:
every second shows something real while a voice explains it. What goes wrong is rarely the
idea; it is layers colliding, a title still on screen after its shot ended, a word the voice
mispronounces, a capture that stutters, a render that cannot be rebuilt the night before the
deadline. These rules come from two finished productions (a 1:55 pitch with eleven 3D scenes
and a web app shown on screens inside them, and a 2:48 desktop-app demo with synthetic
narration exported at 4K60).

## 0. Before the first file

- **Output outside the repository**, on the disk with the most space. The repository holds
  code, small textures and documents; renders, captures and voice takes live elsewhere.
- **Keys in files only the owner can read**, never in the chat, the code or a log. Test what
  each key may do before you depend on it (a voice key may synthesise but not list voices or
  read its quota).
- **Find the tools already installed.** A machine often has several ffmpeg builds (from
  Kdenlive, CapCut, OBS) and they differ: encoders, libass, NVENC version.
- **Set the Git author** before the first commit.

## 1. One cut table

Keep one file (`scenes.json`, or one list at the top of the cut script) that holds every shot:
start, end, source, in-point, which screens it has and the stable frontal stretch; every
overlay with start, end and position; every voice line with its window. Render, screen
inserts, audio, subtitles, export and the editor's timeline (FCPXML) all read from it. When the
same time is written in two scripts, one of them will be wrong.

## 2. The script

- **For the voice: text and a maximum time per part**, nothing else. No stage directions.
- **Two fields per line: what the subtitle says and how it is spoken** (`text` / `say`).
  "ArchieOS" is said "Archie, oh-ess"; "RTX 3070" is said "R T X thirty seventy".
- **So it does not sound written by a model:** no "you don't need X, you need Y", no slogans
  in the negative ("NOT A SCREENSHOT BOT"), no lists of three, no long dashes. Each line says
  one concrete thing that is **on screen at that moment**. Numbers carry their honest caveat
  ("in our development runs").
- **If the voice says it, the title is redundant.** A slogan that repeats the narrator
  competes with the voice.
- **Provenance goes in the description, not on screen.** What is a real capture, what is a
  visualisation, what is sped up: write it in the video description and the gallery, and keep a
  table in the README. One-second tags on screen pass too fast and are misread.

## 3. Design the frame before any graphic

- **Reserve the subtitle band.** At 1080p it sat at y 800-885 (bottom margin 204): no graphic
  goes below y 770, and the band stays above the dock and the chips of the recorded interface.
- **Titles that coincide with a narrator line go top left**; titles with no narration may
  sit lower.
- **Design each graphic with its content on top** (cards, boxes, figures) and its chips at the
  foot. Moving them later costs a render.
- **If a graphic covers a real take, move the take, not the graphic.** Shift the capture
  (e.g. 460 px right, its left side faded into the background colour) and put the metrics
  beside it.
- **Groups move together.** Objects that travel as a group leave at the same time; 0.2 s
  apart, chips stacked diagonally on top of each other. Rows are spaced more than an object's
  height, more still under a tilted camera. Fixed labels stay out of the path of anything that
  moves.

## 4. Layers and the cut

Build the cut from the table: one clip per shot, concatenated, then layers on top in this
order: dark scrims, overlays, URL, and at export the subtitles.

Every overlay accepts an offset (`dx`, `dy`) to move it into free space and an `end` (0.3 s
fade, then trimmed). **No title crosses a shot cut unless you decided it**: a six-second
headline that started four seconds before a cut sat on top of the next shot.

## 5. Real screens

Record the real product (see `screen-capture`). Never simulate an interface in 3D or in a
graphic; never record against a live API (a demo mode replays recorded data). Record exactly
the commit you present.

## 6. Voice, music, captions

See `voice-and-captions` and `sound-design`. Voice lines start inside their shot's window and
end before it closes; music ducks under the voice; the mix lands at -16 LUFS with true peak
under -1.5 dBTP; captions sit in the reserved band.

## 7. Review frame by frame

See `frame-review`. A script flags collisions on every frame; contact sheets every 0.5 s show
the whole video at a glance; you look closely only where the script points.

## 8. Deliver

See `delivery`: build at 1080p60, scale to 3840x2160 at the end, draw subtitles at 4K, check
with ffprobe. Also export an FCPXML and per-scene clips so a designer can redo the cut.

## 9. A rebuild that works without you

- **A `.bat` (or shell script) on the desktop** rebuilds everything from the finished pieces,
  in order: voice, music and subtitles; graphics and screen composites; cut, export and
  review. No paid API calls (voice takes are on disk), a log in the export folder. If the
  tokens or the help run out, the video can still be rebuilt.
- **Each stage skips what is already done and valid** (a file of the right size and
  duration), so a crash does not mean starting over.
- **Before a long run, parse every script** (`python -c "import ast,sys; ast.parse(open(sys.argv[1]).read())" f.py`).
  Three runs broke on: `\n` inside bash heredocs that reached Python as real newlines; Windows
  paths passed through `sed` (`\b`, `\f` became control characters); bash's `$` inside a
  PowerShell command.
- **Change code with an editor**, never with `sed` over paths or backslashes.

## 10. Closing

- Revoke temporary keys (VPN, remote access), stop remote agents.
- Search the commit for keys; scripts that hold keys are in `.gitignore`.
- Identify which remote is the official one and check on the host that the commit arrived.
  If the demo deploys from a fork, push there too.

## In editemos

- `editemos init "<name>" --size 1920x1080` and `editemos kinds use product-pitch`.
- Titles, lower thirds and callouts from the motion library (`editemos_motion`), rendered with
  alpha, reviewed at every settle point; they read the edit brand, not a default.
- `editemos_reference` on a pitch you admire, then `compare` on your cut.
- `editemos_deliver` for the final master and its ffprobe check.
