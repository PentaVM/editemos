---
name: reverse-engineer-a-reference
description: How to imitate an edit the user points at. Measure it, read what cannot be measured, state the unknowns, build, then verify against the reference instead of assuming. Read when the user says "make it like this".
---

# Reverse-engineering a reference

"Make it like this video" is a specification. Treat it like one: take it apart with
evidence, write down what you could not determine, rebuild, and check the rebuild against
the original.

## 1. Measure

`editemos_reference file=<video or link> name=<short name>` measures:

- **Rhythm:** every hard cut, the median shot length, the fast (10%) and slow (90%) ends, how
  much shot length varies from one shot to the next, cuts per minute.
- **Grade:** brightness, saturation, warmth, contrast and palette, per shot and overall.
- **Loudness:** integrated LUFS and loudness range.
- **Sheet:** one frame per shot.

Each number comes with how it was measured (`evidence`).

## 2. Read what numbers miss

Look at the sheet and around the cuts and write down, in words:

- Type: family style, weight, case, where it sits, how it enters.
- Graphics: shapes, colours, how they move (snappy, floaty, on the beat).
- Camera: sizes, movement, handheld or locked.
- Transitions other than hard cuts (the measurement only finds hard cuts).
- Why cuts happen: on action, on the beat, on the line.

## 3. State the unknowns

List what neither the numbers nor your eyes settled (music tempo, the exact font, whether a
move is a spring or a curve). Ask the user only about the ones that change the result.

## 4. Rebuild

Adopt the rhythm (`action=apply` writes the tempo and a cutting rule into the edit brand),
plan the cut with those shot lengths, grade toward the measured look, write motion that
matches what you read.

## 5. Verify

`editemos_reference action=compare reference=<name> file=<our cut>` measures our cut the same
way and lists, trait by trait, where it matches and what to change ("median 3.1 s against
1.4 s: cut sooner"). Fix what matters for this edit and compare again. Matching numbers are
necessary, not sufficient: also watch both side by side.

Never claim a cut "matches the reference" without the comparison.
