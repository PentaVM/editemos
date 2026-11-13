---
name: delivery
description: Exporting and publishing. Lossless intermediates, a 1080p60 build scaled to a 4K60 YouTube master with subtitles drawn at 4K, exact encoder settings and an ffprobe check, per-scene clips and an FCPXML for the editor, Devpost gallery and YouTube channel art sizes, and the closing checklist. Read before exporting a finished video.
---

# Delivery

## Intermediates

- **Lossless until the end** (PNG, ProRes 4444, FFV1 or x264 `-qp 0`). Compress once, at the
  last step.
- **A changed scene gets a new frame folder.** Renderers skip PNGs that already exist (that is
  how they resume), so they will happily reuse stale frames. Move the old folder away first.
- **Every stage skips what is done and valid**: a file with the right size and duration.

## The YouTube master (4K60 from a 1080p60 build)

Build and review at 1080p60. At the very end, scale with lanczos to 3840x2160 and **draw the
subtitles at 4K** (libass draws at the output resolution, so they come out sharp; drawn at 1080p
and scaled, they come out soft). YouTube gives 4K uploads a better codec and bitrate, so even a
1080p cut looks better delivered at 4K.

```bash
ffmpeg -i cut-1080p60.mov -vf "scale=3840:2160:flags=lanczos,subtitles=captions.ass" \
  -c:v libx264 -preset slow -profile:v high -crf 16 -maxrate 68M -bufsize 136M -g 30 -bf 2 \
  -pix_fmt yuv420p -color_primaries bt709 -color_trc bt709 -colorspace bt709 \
  -c:a aac -b:a 384k -ar 48000 -movflags +faststart master-4k60.mp4
```

Then check it, do not assume it:

```bash
ffprobe -v error -show_entries stream=codec_type,width,height,r_frame_rate:format=duration -of json master-4k60.mp4
```

3840x2160, `60/1`, the exact duration (frames / 60), an audio stream present. A 2:48 master
with x264 `slow` took about 8 minutes and weighed 328 MB.

`editemos_deliver file=<cut> preset=youtube-4k subtitles=<.srt|.ass>` does both steps and
returns the check.

## Loudness at delivery

- Narrated pitch, YouTube: -16 LUFS integrated, true peak under -1.5 dBTP, music ducked under
  the voice.
- Shorts, reels, stream clips: -14 LUFS (what `clips build` delivers).
- Broadcast: -23 LUFS (EBU R128) / -24 LKFS (ATSC).

## For the editor and the designer

- **Per-scene clips:** 1080p60, trimmed to the exact time, with voice and subtitles, plus the
  `.srt` and one zip. Check the total frame count (6900 frames for 1:55 at 60 fps), resolution
  and audio of every clip.
- **An FCPXML** generated from the cut table, each piece on its own track, so the cut can be
  redone in DaVinci Resolve or Final Cut. `clips` also writes an EDL.
- **A separate `.srt`** next to every video with burned captions.

## Galleries and channel art

- **Devpost:** 3:2 images at 1800x1200 under 5 MB. Screenshots are cropped; diagrams get
  margins so no text is cut. A GIF of the key moment holds 2.5 s on its last frame. Each image
  gets a caption under 140 characters.
- **YouTube description:** what is a real capture and what is a visualisation, links, credits
  and licences.
- **YouTube banner:** 2560x1440, logo and line inside the 1546x423 safe area in the centre.
  No text against the edges of the video either.

## Licences

Music and effects with a licence, written into `LICENSES_AND_ASSETS.md` from day one. Temporary
music sits on its own track so it can be replaced.

## Closing checklist

1. ffprobe on every delivered file.
2. Revoke temporary keys (VPN, remote desktop, render agents) and stop remote agents.
3. Search the commit for keys; scripts with keys are in `.gitignore`.
4. Identify the official remote and confirm on the host that the commit arrived; if a demo
   deploys from a fork, push there too.
5. Only then commit and push.
