# Architecture

One Rust crate, one binary, no async runtime, no build scripts.

```
            Claude Desktop / Claude Code / Codex / Gemini / Cursor ...        you
                              |  MCP (stdio, JSON-RPC)                         |  terminal
                              v                                                v
                        src/mcp.rs  ------------------+------------------  src/main.rs
                                                      |
                                                 src/ops.rs            JSON in, JSON out
                                                      |
   +-----------+-----------+-----------+-----------+--+--------+-----------+-----------+
   |           |           |           |           |           |           |           |
 store      memory      extract      brand       motion      clips     kdenlive     skills
 projects   JSONL+BM25  native       edit brand  templates   loudness  inspect      catalog
 on disk    context     browser(CDP) derivation  lint        + pauses  diff         sync+scan
            pack        scrapling                render      export    overlay
                                                 library               learn
                                                      |
                                   session (agent in its own window, built-in chat)
                                   studio  (live local page)        adapters (any model)
```

## Rules that hold everywhere

1. **`ops` is the product.** The command line and the MCP server are translations of the same
   functions, so a person and an agent always get the same behaviour.
2. **The project folder is the memory.** `~/.editemos/projects/<slug>/` holds `project.toml`,
   `brand.json`, `memory.jsonl`, `assets/`, `motion/` (scenes, renders, `library.json`), `clips/`,
   `context/` and `runs/`. Everything is plain JSON, TOML and JSONL: readable, diffable, yours.
3. **Deterministic where it matters.** The edit brand is a pure function of the brand and the
   project's name. A scene's key is a hash of template, parameters, size and edit brand. The same
   request therefore finds the same file instead of producing another.
4. **External tools are optional and never bundled.** A Chromium-based browser (motion,
   JavaScript sites), ffmpeg (encode, clips), Kdenlive/melt (timeline), Scrapling (hard-to-read
   sites). `editemos doctor` says what is present; each feature degrades on its own.

## Motion pipeline

`templates/*.html` + edit-brand tokens -> a self-contained scene file. The renderer starts the
machine's own browser headless, opens one window per worker, seeks the scene to each frame time
through `window.__motion.seek(t)`, screenshots with a transparent background, and hands the PNG
sequence to ffmpeg (ProRes 4444, QuickTime Animation, PNG-in-MOV or VP9, all with alpha). Motion
blur is real: several sub-frames per frame are averaged in premultiplied alpha.

The linter reads the tweens the scene actually declared and flags the defaults every tool and
every model reaches for: stock easings, one duration for everything, fade plus small slide,
bounce, default fonts.

A scene is either a timeline (`Motion.timeline().fromTo(...)`) or a Remotion-style
composition (`Motion.composition({durationInFrames, fps, render(frame)})` with `interpolate`,
`spring`, `Sequence`, `Series`, `Easing`). Before the report is read, a composition is walked
frame by frame once, so every `interpolate`/`spring` call site becomes one tween the linter can
judge, and every sound cue inside a `Sequence` is registered at its absolute time.

Sound cues (`report.sfx`) are synthesised in Rust (`motion/sound.rs`: seeded noise, filters and
oscillators, 48 kHz stereo), audio files a scene places are mixed by ffmpeg, and the result is
muxed into the render without re-encoding the picture.

Review: from the tweens the renderer computes the moments the scene comes to rest (a couple of
frames after each move ends, merged when close, plus the last frame). At each one it asks the
page to inspect its text (off the frame, outside the safe margin, overlapping, too small, low
contrast) and puts one still per moment in `review.png` for the agent to judge.

## Reference pipeline

`reference.rs` decodes the whole video once at a low rate into 64x36 frames (in memory, raw
RGB), detects hard cuts with ffmpeg's scene score, takes each shot's middle frame for its
palette and grade, and reads loudness with `ebur128`. Comparison applies the same measurement
to a candidate and checks each trait against a tolerance, phrased as what to change.

## Clips pipeline

One audio-only ffmpeg pass yields momentary loudness every 100 ms (EBU R128) and every silence.
Candidate clips start where a pause ends and end where one begins; each is scored against the
recording as a whole (level, variation, share of the loudest tenth, a hook in the first three
seconds, and words when a transcript is given). The best non-overlapping ones are exported in
parallel, reframed by scale-to-cover and centre crop.

## Why no plugin inside Kdenlive

Kdenlive has no plugin API for external automation and, on Windows and macOS, no live IPC.
editemos therefore works beside it: it reads the saved project, writes versioned copies, hands
back media with exact timeline positions, and learns from the difference between two saves.
On Linux Kdenlive exposes D-Bus; using it for live control is on the roadmap.
