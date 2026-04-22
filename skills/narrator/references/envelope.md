# Envelope protocol

Every `narrator-cli` invocation prints **exactly one line of JSON** on stdout. This doc defines the shape and shows how to consume it.

## Success envelope

```json
{"ok": true, "data": { /* subcommand-specific payload */ }}
```

`data` is always an object. Its shape depends on the subcommand — see [subcommands.md](subcommands.md).

## Error envelope

```json
{"ok": false, "error": {"kind": "ErrorVariant", "message": "human-readable"}}
```

- `kind` is a stable identifier derived from the Rust `NarratorError` variant (e.g. `FfmpegFailed`, `IoError`, `InvalidInput`). Use it for branching logic.
- `message` is a human-readable description. Show this to the user; don't parse it.

Exit code: `0` on success, `1` on error. Both cases still print a single-line envelope on stdout, so you can always parse the last line.

## Progress events (optional)

Pass `--progress json` (global flag, goes before the subcommand) to get NDJSON progress events on **stderr** — one object per line — while a long-running render is in flight. Stdout stays reserved for the final envelope.

Example:

```bash
narrator-cli --progress json render burn-subs --input in.mp4 --srt in.srt --output out.mp4 \
  2> progress.ndjson
```

Each line of `progress.ndjson` looks like:

```json
{"kind":"progress","percent":42.5,"message":"Burning subtitles"}
```

Fields:

- `kind` — always `"progress"` for this channel today.
- `percent` — float 0.0–100.0.
- `message` — optional; a short human-readable sub-label. May be absent or null.

Progress cadence is not guaranteed. Treat it as best-effort; never rely on a specific tick frequency.

## Consuming the envelope

### Shell + jq

```bash
out=$(narrator-cli probe video --input video.mp4)
ok=$(echo "$out" | jq -r '.ok')
if [ "$ok" != "true" ]; then
  echo "error: $(echo "$out" | jq -r '.error.message')" >&2
  exit 1
fi
echo "$out" | jq '.data'
```

### Streaming progress + capturing final envelope

```bash
narrator-cli --progress json render apply-edits \
  --input in.mp4 --plan plan.json --output out.mp4 \
  2> >(while read -r line; do echo "[progress] $line"; done >&2) \
  | jq '.'
```

### Node.js (parsing the envelope)

```js
import { spawnSync } from "node:child_process";
const r = spawnSync("narrator-cli", ["probe", "video", "--input", "video.mp4"], { encoding: "utf8" });
const env = JSON.parse(r.stdout.trim());
if (!env.ok) throw new Error(`${env.error.kind}: ${env.error.message}`);
console.log(env.data);
```

### Python (parsing the envelope + streaming progress)

```python
import json, subprocess

# One-shot: capture the final envelope.
r = subprocess.run(
    ["narrator-cli", "probe", "video", "--input", "video.mp4"],
    capture_output=True, text=True, check=False,
)
env = json.loads(r.stdout.strip())
if not env["ok"]:
    raise RuntimeError(f"{env['error']['kind']}: {env['error']['message']}")
print(env["data"])

# Long-running: stream NDJSON progress on stderr, keep stdout for the envelope.
with subprocess.Popen(
    ["narrator-cli", "--progress", "json", "render", "burn-subs",
     "--input", "in.mp4", "--srt", "in.srt", "--output", "out.mp4"],
    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
) as p:
    for line in p.stderr:                       # NDJSON progress events
        evt = json.loads(line)
        print(f"{evt['percent']:.0f}%  {evt.get('message', '')}")
    envelope = json.loads(p.stdout.read().strip())
```

## Input handling

Subcommands that take a JSON body accept either:

- `--input /path/to/file.json` — read from a file.
- `--input -` — read from stdin.

This applies to `render apply-edits --plan`, `render burn-subs --style`, and `tts narrate --script`. All other arguments are plain strings/numbers.
