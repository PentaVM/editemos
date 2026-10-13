# Security

## Reporting

Please report vulnerabilities privately through GitHub's "Report a vulnerability" button on this
repository (Security tab), not in a public issue.

## What editemos does and does not do

- **Local store.** Projects, memory, brand and assets live under `~/.editemos` (or
  `EDITEMOS_HOME`). Nothing is uploaded.
- **API keys** are read from environment variables at call time and never written to disk,
  logs or memory.
- **The studio** binds to `127.0.0.1` only, serves only files the project knows about, and
  rejects requests whose `Origin` is another site.
- **Website extraction** obeys `robots.txt` unless you pass `--ignore-robots`, and identifies
  itself with an honest user agent.
- **Third-party skills** are downloaded as text only, pinned to a commit, hashed, and scanned
  for prompt-injection patterns (instruction overrides, concealment from the user, pipe-to-shell,
  decode-and-run, secret exfiltration, hidden Unicode). Flagged files are kept out of exports.
  The scanner is a heuristic, not a guarantee: skill text is treated as data, and you should
  read what you enable.
- **Kdenlive projects** are never modified in place; edits go to versioned copies.
- **Sessions** started with `editemos session` run the agent you chose with that agent's own
  permission model. editemos does not grant it anything extra.

## Supported versions

The latest release.
