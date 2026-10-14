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

## Loudness

- Streaming platforms normalise to about -14 LUFS integrated (some -13 to -16). Deliver near
  it so the platform does not turn you down or up.
- Broadcast: -23 LUFS (EBU R128) or -24 LKFS (ATSC).
- True peak below -1 dBTP so encoding does not clip.
- Measure (`reference` reports integrated loudness and range); do not judge by ear on laptop
  speakers.
