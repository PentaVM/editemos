# Edit kinds: open types of edit, and the data they learn from

An **edit kind** is a type of edit written down so anyone can use it, improve it and train a
model on it: gameplay highlights, a talking story clip, a calm documentary, a fighting-game
combo reel, a wedding teaser, a product launch. editemos ships three; everything else comes
from people who edit that kind of video.

A kind is a folder of text:

```text
kinds/<id>/
  kind.toml        what it is, the rules an editor of it follows, defaults, an adapter
  examples.jsonl   edits people approved, corrected or rejected (one per line)
```

It works with any model the moment it exists: when a project uses a kind
(`editemos kinds use <id>`), its rules and a couple of approved examples go into the context
every agent reads, and its defaults go into `clips build`. When enough examples exist, a LoRA
adapter trained on them can be published, and a local model planning with that adapter is
based on the kind itself.

## Write one

```bash
editemos kinds new "Fighting game combos" --from gameplay-highlights
# edit ~/.editemos/kinds/fighting-game-combos/kind.toml
editemos kinds use fighting-game-combos        # in a project that makes them
editemos kinds validate fighting-game-combos   # ready to share?
```

`kind.toml`:

```toml
id = "fighting-game-combos"            # lower-case, dashes; the folder has the same name
name = "Fighting game combos"
description = "What this edit is, who watches it and what makes one good."
version = "1"
license = "CC0-1.0"                    # CC0-1.0, CC-BY-4.0, MIT or Apache-2.0: it is training data
authors = ["your-name"]
tags = ["gaming", "fgc", "vertical"]
rules = [                              # what an editor of this kind always and never does
  "Start on the opening hit, not on the neutral before it.",
  "Hold the last hit of the combo until the health bar settles.",
]

[defaults]                             # anything clips build accepts; all optional
aspect = "9:16"
layout = "fit"
transition = "zoom"                    # zoom | whip | dip | flash | cut
breath = 0.0
max_pause = 0.0
captions = false
sfx = true
clip_seconds = 45.0
score_region = "0.405,0.012,0.04,0.06;0.552,0.012,0.04,0.06"   # when every recording shares a HUD

[target]                               # what a good one measures like (editemos reference)
median_shot = 2.5
loudness_lufs = -14.0
```

Rules are editing rules, written for an editor. Text that reads like instructions to an agent
(hide this from the user, ignore previous instructions, send keys somewhere) is refused by
`validate`, with the same scanner that checks downloaded skills.

## Record examples

Every clip a person judges is an example. After `clips build`, each clip has a
`<name>.story.json` (the plan) and a `<name>.build.json` (the cut, the goals and impacts it was
built around, the checks). Then:

```bash
editemos dataset add clips/the-play.mp4 --verdict approved --notes "the impact lands"
editemos dataset add clips/the-play.mp4 --verdict corrected --correction wanted.json --notes "end after the goal"
editemos dataset add clips/the-play.mp4 --verdict rejected --notes "nothing happens"
editemos dataset stats
```

An agent does this as soon as you say what you think of a clip.

**Examples never contain footage.** They point at their source by link and section:

```json
{"id": "...", "kind": "gameplay-highlights", "verdict": "corrected",
 "source": {"url": "https://www.twitch.tv/videos/...", "section": "01:40:00-02:00:00", "offset": 6000.0, "sha256_head": "...", "duration": 1200.0},
 "window": [398.8, 659.0],
 "plan": {"name": "...", "segments": [{"start": 418.8, "end": 423.1, "label": "2:19  3-3"}]},
 "correction": {"...": "the plan the person wanted"},
 "cut": [[418.8, 423.2]], "events": [{"t": 423.25, "what": "goal", "team": 1}],
 "notes": "end after the outcome"}
```

Times are seconds in the local recording; add `offset` for seconds in the linked video.
`clips` records the link when it fetches one; for a file you fetched yourself,
`editemos dataset source <file> --url <page> --section <part>` records it. Only share
examples of footage the creator allows to be used this way, and say whose it is in `notes` if
the licence asks for attribution.

## Export a training set

```bash
editemos dataset export --out ./train --fps 0.5 --max-frames 48
```

writes

- `sft.jsonl`: one line per approved or corrected example. `messages` has the user turn (one
  `<image>` per frame, then the task: the kind's rules, the times the frames were taken at, and
  the plan format) and the assistant turn (the plan, or the correction). `images` lists the
  frames in order. This is the messages-with-images layout most vision-language fine-tuning
  tools read; map the field names if yours expects others.
- `preference.jsonl`: one pair per corrected example (`chosen` = the plan the person wanted,
  `rejected` = the plan that was built) and per rejected example (`chosen` = no clip), for DPO
  or any preference method.
- `frames/<example>/<seconds>.jpg`: 448 px wide, taken from the recordings on your machine
  (recognised by the hash of their first 8 MB). Frames are always taken at the events (goals,
  impacts) as well as at the regular rate, so a model sees the moments an edit turns on.

The recordings themselves are not part of the set. Do not publish `frames/` with it.

## Train an adapter

Any LoRA trainer for a vision-language model will do. A recipe that fits on one 24 GB GPU:

1. A small vision-language base that reads several images and writes JSON well, at 7B or less.
2. LoRA rank 16 to 32 on the language model's attention and MLP projections, vision tower
   frozen, 2 to 3 epochs over `sft.jsonl`, learning rate around 1e-4.
3. Optionally a DPO pass over `preference.jsonl` starting from the result.
4. Hold out some examples (different sources from the training ones) and check the plans:
   does every segment end after its outcome? `clips build` with `--score-region` flags the
   ones that do not.

Then publish the weights (a release asset, a model hub) and add to `kind.toml`:

```toml
[adapter]
base = "the base model id"
format = "gguf"                       # gguf (llama.cpp, Ollama) or safetensors (PEFT)
url = "https://.../fighting-game-combos-lora.gguf"
sha256 = "..."
note = "rank 16, 140 examples from 31 sources"
```

`editemos kinds adapter <id>` downloads it, checks the hash and prints how to load it
(llama.cpp `--lora`, an Ollama Modelfile with `ADAPTER`, or PEFT).

## Share it

Open a pull request that adds your folder under `kinds/` in this repository. Before you do:

- `editemos kinds validate <id>` passes (licence, rules, text only, examples well formed);
- every example's source has a link;
- the rules describe editing, in your own words.

Everyone else gets it with `editemos kinds sync`; once merged, the next release ships it.

## Status

The kinds, the dataset, the export and the adapter download are in place and tested. No
adapter has been trained yet: the first ones need examples, which is what this is for.
