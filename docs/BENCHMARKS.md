# Benchmarks

Every number here comes from `editemos bench`, which runs in a throwaway home directory and
prints this table. Run it on your machine; `--quick` takes about a minute.

```bash
cargo build --release
./target/release/editemos bench
```

## Reference run

editemos 0.1.0, release build, Windows 11, 6 logical cores, Edge headless, ffmpeg from the
Kdenlive bundle. A laptop, not a workstation.

Two runs are shown because they differ and hiding that would be dishonest: **busy** was taken
while an unrelated compile job was using the CPU and the machine was short of memory;
**quiet** was taken earlier the same day with the machine mostly idle.

| What | Busy | Quiet | How it is measured |
| --- | ---: | ---: | --- |
| start the binary | 19 ms | 19 ms | `editemos --version`, median of 10 |
| create a project with its edit brand | 58 ms | 56 ms | folders, config, derived tokens, context pack |
| write a memory | 0.60 ms | 0.33 ms | average over 5,000 appends |
| search memory (BM25) | 12 ms | n/a | 5,000 memories, top 8 (the quiet run predates an optimisation: 97 ms) |
| build the context pack | 20 ms | 47 ms | ~830 tokens ranked out of 5,000 memories |
| brand analysis of a page | 6.1 ms | 6.8 ms | HTML + CSS to palette, fonts, logo, voice; network excluded |
| motion capture, 1 window | 9.1 fps | 7.7 fps | 1920x1080 transparent PNG frames out of the browser |
| motion capture, 3 windows | 9.7 fps | 11.3 fps | same, three browser windows in parallel |
| ask for a graphic that exists | 11 ms | 21 ms | returned from the motion library, nothing rebuilt |
| clips: listen to a recording | 226x | 736x | 60 minutes of audio, loudness every 100 ms plus every pause |
| clips: choose the moments | 9.6 ms | 5.7 ms | 8 clips from 36,000 samples and 515 pauses |

Binary size: 8.8 MB, one file, no runtime.

## What the numbers do and do not say

- **Motion capture is bounded by the browser**, not by Rust: each 1080p frame is a real
  screenshot of a real page. Parallel windows help when cores are free (11.3 vs 7.7 fps quiet)
  and barely at all when they are not (9.7 vs 9.1 busy).
- **Encoding used to dominate.** ProRes 4444 with 16-bit alpha and rate control took 8.7 s for
  60 frames on this machine; 8-bit alpha at fixed quality takes 1.0 s for the same frames, and
  that is now the default. `--format qtrle` (1.2 s) and `--format pngmov` (1.6 s) are lossless.
- **Browser start-up varies wildly** with machine load: 0.8 s to 15 s observed on this laptop.
  The wall-clock time of a short render is mostly that.
- **Clips analysis is audio-only on purpose.** Decoding video to look at the picture would cost
  two orders of magnitude more; a 3-hour stream is listened to in well under a minute.
- **Extraction over the network** is not benchmarked here because it measures the site.
  Observed: 8 to 23 s for real sites, almost all of it downloads, which run eight at a time.

## Real sites tried during development

| Site | Backend | Time | Result |
| --- | --- | ---: | --- |
| kdenlive.org | native | 23 s | 12 assets, palette and typefaces found (was 105 s before downloads ran in parallel) |
| linear.app | auto | 18 s | plain HTTP showed no typeface, so it re-read the rendered page: Inter Variable, dark palette |
| kdenlive.org | scrapling | 30 s | same brand as native, through the Scrapling CLI |
