---
name: open-source-stack
description: Twenty open-source tools for animation, physics, 3D, sound and delivery (Remotion, HyperFrames, Anime.js, React Spring, PixiJS, p5.js, Paper.js, Matter.js, Three.js, Rapier, OGL, tsParticles, glTF Transform, GL Transitions, OBS Studio, Chatterbox, Tone.js, faster-whisper, OpenCut, FFmpeg), what each is for, and how to make each one render frame-exact inside an editemos scene. Read when a piece needs more than titles and lower thirds.
---

# The open-source stack

Animation. Physics. 3D. Sound. Delivery. Brief, prototype, animate, render, ship.

editemos bundles none of these. A scene is an HTML page; vendor a library's `.js` file next to
the scene and use it there. Check each project's licence before you ship its code inside a
deliverable (Remotion, for one, has its own licence that companies above a size must pay for).

## The one rule: the frame is a function of time

editemos renders a scene by seeking it to each frame, in several browser windows at once, in
any order. A library that runs its own clock (`requestAnimationFrame`, a ticker, `setTimeout`)
will not reproduce the same frame twice. So for every library: **stop its clock, and draw
frame `n` from `n` alone** inside `Motion.composition({ render(frame) })` or a timeline
`.call(fn)`. Randomness comes from a seed (`random(seed)` or the library's own seed function).
Stateful simulations are computed once from frame 0 and read back per frame.

## Animation

| # | Tool | For | In an editemos scene |
| --- | --- | --- | --- |
| 01 | [Remotion](https://github.com/remotion-dev/remotion) | React video engine | Its API is already here without React: `interpolate`, `spring`, `Sequence`, `Series`, `Easing` (guide `motion-remotion`). Use Remotion itself when the project is a React codebase. |
| 02 | [HyperFrames](https://github.com/heygen-com/hyperframes) | HTML-to-video renderer | Same idea as editemos motion (HTML seeked frame by frame). Its skills are in the catalogue as `hyperframes` (`editemos skills sync`). |
| 03 | [Anime.js](https://github.com/juliangarnier/anime) | Timeline choreography | Create animations and timelines with `autoplay: false`; in `render(frame)` call `.seek(frame / fps * 1000)`. |
| 04 | [React Spring](https://github.com/pmndrs/react-spring) | Spring animation | Needs React. The built-in `spring({ frame, fps, config })` is the same damped-oscillator physics and seeks exactly; use that. |

## 2D graphics

| # | Tool | For | In an editemos scene |
| --- | --- | --- | --- |
| 05 | [PixiJS](https://github.com/pixijs/pixijs) | GPU 2D graphics, sprites, filters | `backgroundAlpha: 0`, `preserveDrawingBuffer: true`; `app.ticker.stop()`; set positions from the frame and call `app.render()`. |
| 06 | [p5.js](https://github.com/processing/p5.js) | Generative visuals | Instance mode, `noLoop()`, `randomSeed()` and `noiseSeed()` in setup; in `render(frame)` set the time and call `redraw()`. Accumulating sketches must redraw from scratch. |
| 07 | [Paper.js](https://github.com/paperjs/paper.js) | Vector paths, boolean ops, path morphs | No `onFrame`; compute path state from the frame, then `paper.view.update()`. |
| 08 | [Matter.js](https://github.com/liabru/matter-js) | 2D physics | Do not start the `Runner`. At load, step `Engine.update(engine, 1000 / fps)` for every frame and store each body's position and angle; `render(frame)` reads the stored frame. |
| 12 | [tsParticles](https://github.com/tsparticles/tsparticles) | Particle effects | Runs its own clock: good for prototypes and loops, not for a seeked render. For the final, bake it (record once, place the PNG sequence as footage) or write the particles in `render(frame)` with `random(seed)`. |

## 3D and shaders

| # | Tool | For | In an editemos scene |
| --- | --- | --- | --- |
| 09 | [Three.js](https://github.com/mrdoob/three.js) | 3D scenes | `WebGLRenderer({ alpha: true, preserveDrawingBuffer: true })`, no animation loop; `renderer.render(scene, camera)` inside `render(frame)`. Load models, then mount (guide `3d-scenes`). |
| 10 | [Rapier](https://github.com/dimforge/rapier) | 3D collisions and rigid bodies | Deterministic WASM. `world.timestep = 1 / fps`, step from frame 0 once at load, store transforms, read them per frame. |
| 11 | [OGL](https://github.com/oframe/ogl) | Minimal WebGL, custom shaders | A `uTime` uniform from the frame; one draw per seek. |
| 13 | [glTF Transform](https://github.com/donmccurdy/glTF-Transform) | 3D asset optimisation | Before the scene: `npx @gltf-transform/cli optimize in.glb out.glb`. Every render worker loads the model, so size is load time. |
| 14 | [GL Transitions](https://github.com/gl-transitions/gl-transitions) | Shader transitions | Each is a GLSL `transition(uv)` with a `progress` uniform. Draw both shots as textures and drive `progress` with `interpolate(frame, [a, b], [0, 1], { easing })`. Keep them for designed moments: most cuts are hard cuts (guide `cutting`). |

## Capture, voice and sound

| # | Tool | For | How it fits |
| --- | --- | --- | --- |
| 15 | [OBS Studio](https://github.com/obsproject/obs-studio) | Screen capture | Real-time capture when the app cannot be stepped frame by frame. Constant frame rate, lossless or near-lossless, notifications off, the owner's agreement (guide `screen-capture`). |
| 16 | [Chatterbox](https://github.com/resemble-ai/chatterbox) | Voice-overs | Local TTS: narration without sending the script anywhere. One sentence per call, `text` and `say` per line, a Whisper check per line; cloning a voice needs its owner's consent (guide `voice-and-captions`). |
| 17 | [Tone.js](https://github.com/Tonejs/Tone.js) | Synths and sound effects | Real-time Web Audio does not reach a seeked render. Render offline (`Tone.Offline(cb, seconds)`), save the WAV next to the scene, place it with `Motion.sfx(t, 'file', { src })` or `Audio({ src, from })`. For simple cues the built-in synthesised `sfx` kinds are enough. |
| 18 | [faster-whisper](https://github.com/SYSTRAN/faster-whisper) | Caption transcription | `word_timestamps=True` for word times; check every synthetic line; align the script's corrected words to the ASR's times. editemos itself uses whisper.cpp for clips. |

## Editing and delivery

| # | Tool | For | How it fits |
| --- | --- | --- | --- |
| 19 | [OpenCut](https://github.com/OpenCut-app/OpenCut) | Timeline editing | An open-source timeline next to Kdenlive and DaVinci: drop the alpha renders, clips and `.srt` there. editemos works beside editors through files. |
| 20 | [FFmpeg](https://github.com/FFmpeg/FFmpeg) | Encoding and delivery | Already the engine under motion renders, clips and references. Final masters with `editemos_deliver` (guide `delivery`). Check which build you have: libass for subtitles, NVENC version, codecs. |

## Choosing

- A title, lower third, quote, caption: the built-in templates. Nothing else needed.
- Choreography of many elements: timeline or composition; Anime.js if the team already writes it.
- Thousands of sprites, particles, filters: PixiJS. Sketchy, generative, organic: p5.js. Vector
  morphs and boolean shapes: Paper.js.
- Things that fall, collide, pile up: Matter.js (2D) or Rapier (3D), precomputed.
- A product in 3D, a logo with depth: Three.js (+ glTF Transform). A full shader look: OGL.
- A transition the edit is designed around: GL Transitions.
- A real app on screen: OBS or scripted capture, never a simulation.
- Narration: a person, or Chatterbox/ElevenLabs line by line, checked with faster-whisper.
- Delivery: FFmpeg, checked with ffprobe.

Every scene still goes through `editemos_motion lint` and the review stills, whatever library
drew it.
