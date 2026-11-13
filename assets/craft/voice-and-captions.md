---
name: voice-and-captions
description: Narration recorded by a person or synthesised (ElevenLabs, or Chatterbox on your own machine), checked word by word with faster-whisper, fitted to each shot's window, then captions from the corrected script with ASR timings, two lines of 42 characters, a reserved band, a soft box over busy backgrounds and a separate .srt. Read before recording or generating any voice-over or subtitles.
---

# Voice-over and captions

## A voice recorded by a person

1. **Transcribe every take when it arrives** and keep a table: take, part, duration, words
   that differ from the script. You will know which take covers which part without listening
   to all of them again.
2. **Always the same cleaning chain:** rumble filter, background noise, de-essing, dynamics,
   breaths lowered (not cut to silence), -16 LUFS. Measure LUFS per clip.
3. **A configurable maximum pause per take.** Warn when a take does not fit its time: shorten
   the pauses or start it earlier.

The script the speaker gets is text and a maximum time per part. No stage directions.

## A synthetic voice

- **One sentence per request**, sending the previous and next sentences as context
  (ElevenLabs `previous_text` / `next_text`) so the lines sound like one reading.
- **Each line has `text` (what the caption shows) and `say` (how it is spoken).** Brand names,
  model numbers and acronyms live in `say`.
- **Pick the voice by listening.** Generate 3-4 samples of the same lines. If the key cannot
  list voices, use standard voices by their ID.
- **Whisper checks every line.** Transcribe each generated line (faster-whisper with word
  timestamps). When a word comes out wrong ("ArchieOS" came out "RGOS"; "the files and" came
  out "the files in") **change the sentence or its `say`**; do not fight the voice with retries.
- **Each line starts inside its shot's window and ends before it closes.** Avoid stretching
  the tempo; never more than 8%.
- **Keep every take on disk.** Rebuilding the video must not call the API again.
- **On your own machine:** Chatterbox (open source) generates narration locally, nothing leaves
  the computer. Cloning a voice needs the explicit consent of the person it belongs to.

28 lines with ElevenLabs plus a Whisper check took seconds plus about a minute of CPU.

## Captions

1. **The corrected script's text with the ASR's word timings.** ASR text has errors; script
   text has no times. Take the words from the script and the times from faster-whisper
   (`word_timestamps=True`), aligned word by word.
2. **At most two lines of 42 characters**, broken at commas and full stops, fading in and out
   over 120 ms.
3. **In the reserved band** (at 1080p, y 800-885 with a bottom margin of 204), above the dock
   and above any chips of a recorded interface. Nothing else lives there.
4. **Over a busy background, a soft box** instead of a halo: libass `BorderStyle=4` (or 3),
   navy at about 60% opacity. A dotted line crossing a caption with only an outline reads as
   strikethrough.
5. **Always deliver a separate `.srt`** as well: burned-in captions cannot be edited.
6. **Burn them at the output resolution.** libass draws at the size of the frame it is given,
   so draw after scaling to 4K and they stay sharp.

## Short-form captions

Stream clips and shorts use a different style: one or two words lit at a time, in the project's
typeface, transcribed from the clip's own audio. `editemos_clips action=build` does that;
site auto-captions are only used to find moments, never shown.

## In editemos

- `editemos_clips action=transcript transcribe=true` returns words with times (whisper.cpp,
  local).
- `editemos_deliver subtitles=<file.srt|file.ass>` draws them after the final scale.
- Record the voice and caption rules a project settles on with `editemos_memory_add`.
