---
name: generative-video
description: When to use diffusion video models and when not, how to prompt them with camera language, keeping shots consistent, and the mixed route where generated footage carries organic motion and code-drawn layers carry text and UI. Read before generating any footage.
---

# Generative video

Diffusion video models make organic motion (people, cloth, water, light, camera moves through
a space) that code cannot draw. They also invent geometry: text melts, interface corners
warp, logos mutate from frame to frame. Use each for what it does well.

## The routes

- **Code-drawn (editemos motion).** Text, interfaces, charts, logos, titles, anything that
  must be exact. Deterministic, editable, with alpha.
- **Mixed.** Generated footage underneath for organic motion; code-drawn layers on top for
  every word, number and logo. The default when both are needed.
- **Generated only.** Abstract textures, atmospheres, b-roll plates, transitions. Never for
  text or UI.

## Prompting

Write a shot, not a wish. One line each for:

1. **Subject and action.** Who or what, doing what, in one continuous movement.
2. **Camera.** Size, angle, lens feel, movement ("medium, eye level, slow dolly in").
3. **Light and time.** "Low sun, warm rim light, long shadows."
4. **Look.** Palette and texture in the brand's terms ("muted teal shadows, cream
   highlights, fine grain").
5. **What must not appear.** Text, logos, extra people, if the model accepts negatives.

Short shots work best (3 to 8 seconds). Generate several and choose; it is a casting
process, not a render.

## Consistency

- Start from an image (image-to-video) made from a real frame, a brand still or a previous
  generation: the first frame anchors palette, subject and framing.
- Keep a continuity sheet: the subject's description, palette, lens and light, reused word
  for word in every prompt.
- Fix the seed when the tool allows it to make variations of one take.
- Grade generated shots together with real footage so they match.

## Honesty and rights

- Say which shots are generated when the context calls for it (news, documentary, ads with
  real products).
- Do not generate real people's likeness or a brand's trademarks without permission.
- Generated footage is uploaded to the provider: never send the user's material to a service
  without their yes.

## In editemos

`editemos_generate` makes images with the adapter the user configured (keys from
environment variables). Use those images as first frames for whichever video model the user
has, then bring the clips back into the project as assets and composite code-drawn layers
on top with `editemos_motion`.
