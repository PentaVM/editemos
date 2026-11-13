---
name: sound-design
description: Layers of a mix (dialogue, music, ambience, effects), ducking, sound bridges, risers and hits around cuts, silence as an accent, platform loudness targets, and synthesised motion sound. Read when an edit or a graphic needs sound.
---

# Sound design

Viewers forgive a soft image; they leave over bad sound. Half of what feels like "good
editing" is sound.

## The layers

1. **Dialogue / voice.** Always intelligible. Everything else makes room for it.
2. **Music.** Sets tempo and emotion. Duck it 8-14 dB under speech; bring it up in the gaps.
3. **Ambience / room tone.** Continuous bed under a scene. Cutting it with the picture makes
   every cut audible; keep it running across cuts in the same place.
4. **Effects.** Whooshes, hits, clicks, risers: they give weight to motion and cuts.

## Around cuts

- **Sound bridge.** Start the next scene's sound early (J cut) or let this one run over
  (L cut). Smoother than any visual transition.
- **Riser into a cut.** Tension that lands exactly on the cut. It must end on the cut, not
  start on it.
- **Hit on the cut.** A low impact on a hard cut to a big moment. Use sparingly: three hits
  in a minute and none of them land.
- **Silence.** Dropping everything for half a second before a payoff is louder than any
  effect.

## Motion graphics

Every element that moves on screen can carry a sound: an entrance (whoosh), a landing (thud,
pop), a click on a UI element, a type-on (type). In editemos scenes put the sound on the
tween (`sfx: 'whoosh'`, or `{ kind: 'thud', at: 'end' }`) so it moves with the motion when
the timing changes. Keep cues on a beat grid when there is music (`Motion.beat(n, bpm)`).
Fewer, better: one sound per idea, not per element.

In a product demo or pitch, effects go **only on real events** in the picture: a click in the
take, the moment the agent takes control, the answer arriving. A whoosh on every title is noise.

## Narration and music

- The voice leads; music ducks under it with a sidechain compressor (or keyframed -8 to -14 dB),
  and comes back up in the gaps.
- A recorded voice always goes through the same chain: rumble filter, noise reduction,
  de-essing, dynamics, breaths lowered rather than cut, -16 LUFS. Measure each clip.
- Synthetic narration, one line per request with the lines around it as context, checked by
  transcription (guide `voice-and-captions`).
- Music and effects with a licence, logged in a `LICENSES_AND_ASSETS.md` from the start. A
  temporary track stays on its own track until it is replaced.
- Synths and designed stingers can come from code: Tone.js rendered offline to a WAV and placed
  as a `file` cue (guide `open-source-stack`).

## Loudness

- Streaming platforms normalise to about -14 LUFS integrated (some -13 to -16). Deliver near
  it so the platform does not turn you down or up.
- A narrated pitch or demo on YouTube: -16 LUFS integrated works well (speech-led, room for
  the voice); short clips: -14 LUFS (what `clips build` delivers).
- Broadcast: -23 LUFS (EBU R128) or -24 LKFS (ATSC).
- True peak below -1.5 dBTP so encoding does not clip.
- Measure (`reference` reports integrated loudness and range); do not judge by ear on laptop
  speakers.
