#!/usr/bin/env bash
# Comprehensive perf + behaviour suite for the narrator-cli binary.
#
# Generates a realistic 1080p 30s test video with audio, then runs every
# non-AI subcommand with each significant scenario. For each run we capture:
#   - wall clock (s)
#   - peak RSS (KB)
#   - exit code
#   - output file size (bytes)
#   - ffprobe-validated codec / dimensions / duration / channels
#   - JSON envelope shape (ok=true|false, error.kind if failed)
#
# Output is a markdown table written to scripts/cli_perf_report.md so we can
# diff it across phases and spot regressions.
#
# Usage:  scripts/cli_perf_suite.sh
# Reqs:   ffmpeg, ffprobe, jq, /usr/bin/time -l (macOS) OR /usr/bin/time -v (Linux)

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CLI="$ROOT/src-tauri/target/release/narrator-cli"
REPORT="$ROOT/scripts/cli_perf_report.md"
TMPROOT="$(mktemp -d)"
trap 'rm -rf "$TMPROOT"' EXIT

if [[ ! -x "$CLI" ]]; then
  echo "narrator-cli release binary missing at $CLI" >&2
  echo "Build with: cargo build --manifest-path src-tauri/Cargo.toml --release --bin narrator-cli" >&2
  exit 1
fi
for tool in ffmpeg ffprobe jq; do
  if ! command -v "$tool" >/dev/null; then
    echo "Required tool missing: $tool" >&2
    exit 1
  fi
done

# Pick a portable `time` invocation. macOS: /usr/bin/time -l. Linux: -v.
TIME_BIN="/usr/bin/time"
if [[ "$(uname)" == "Darwin" ]]; then
  TIME_FLAGS="-l"
else
  TIME_FLAGS="-v"
fi

# ── Fixture generation ─────────────────────────────────────────────────────
echo "Generating fixtures in ${TMPROOT}..." >&2

VID_1080P="$TMPROOT/src_1080p.mp4"
VID_720P="$TMPROOT/src_720p.mp4"
NARRATION_M4A="$TMPROOT/narration.m4a"
SRT="$TMPROOT/subs.srt"

ffmpeg -y -hide_banner -loglevel error \
  -f lavfi -i "testsrc=duration=30:size=1920x1080:rate=30" \
  -f lavfi -i "sine=frequency=440:duration=30" \
  -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac \
  "$VID_1080P"

ffmpeg -y -hide_banner -loglevel error \
  -f lavfi -i "testsrc=duration=15:size=1280x720:rate=30" \
  -f lavfi -i "sine=frequency=300:duration=15" \
  -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac \
  "$VID_720P"

ffmpeg -y -hide_banner -loglevel error \
  -f lavfi -i "sine=frequency=200:duration=20" \
  -c:a aac -b:a 128k "$NARRATION_M4A"

cat > "$SRT" <<'EOF'
1
00:00:01,000 --> 00:00:04,000
First subtitle line.

2
00:00:05,000 --> 00:00:08,000
Second subtitle line — with em-dash.

3
00:00:10,000 --> 00:00:13,000
Third line: testing colons & ampersands.
EOF

# ── Plans ───────────────────────────────────────────────────────────────
mkdir -p "$TMPROOT/plans"

write_plan() {
  cat > "$TMPROOT/plans/$1.json" <<EOF
$2
EOF
}

write_plan trim '{"clips":[{"start_seconds":0,"end_seconds":5,"speed":1.0,"fps_override":null}]}'

write_plan speed '{"clips":[{"start_seconds":0,"end_seconds":10,"speed":2.0,"fps_override":null}]}'

write_plan multiclip '{
  "clips":[
    {"start_seconds":0,"end_seconds":5,"speed":1.0,"fps_override":null},
    {"start_seconds":10,"end_seconds":15,"speed":1.5,"fps_override":null},
    {"start_seconds":20,"end_seconds":25,"speed":1.0,"fps_override":null}
  ]
}'

write_plan freeze '{
  "clips":[
    {"start_seconds":0,"end_seconds":3,"speed":1.0,"fps_override":null},
    {"clip_type":"freeze","start_seconds":3,"end_seconds":3,"speed":1.0,"fps_override":null,
     "freeze_source_time":3.0,"freeze_duration":2.0},
    {"start_seconds":3,"end_seconds":6,"speed":1.0,"fps_override":null}
  ]
}'

write_plan zoom_pan '{
  "clips":[{"start_seconds":0,"end_seconds":5,"speed":1.0,"fps_override":null,
            "zoom_pan":{"startRegion":{"x":0,"y":0,"width":1,"height":1},
                        "endRegion":{"x":0.25,"y":0.25,"width":0.5,"height":0.5},
                        "easing":"ease-in-out"}}]
}'

write_plan spotlight '{
  "clips":[{"start_seconds":0,"end_seconds":5,"speed":1.0,"fps_override":null}],
  "effects":[{"type":"spotlight","startTime":1,"endTime":4,"transitionIn":0.3,"transitionOut":0.3,
              "spotlight":{"x":0.5,"y":0.5,"radius":0.2,"dimOpacity":0.7}}]
}'

write_plan blur '{
  "clips":[{"start_seconds":0,"end_seconds":4,"speed":1.0,"fps_override":null}],
  "effects":[{"type":"blur","startTime":0.5,"endTime":3.5,
              "blur":{"x":0.25,"y":0.25,"width":0.5,"height":0.5,"radius":15,"invert":false}}]
}'

write_plan text '{
  "clips":[{"start_seconds":0,"end_seconds":4,"speed":1.0,"fps_override":null}],
  "effects":[{"type":"text","startTime":0.5,"endTime":3.5,"transitionIn":0.2,"transitionOut":0.2,
              "text":{"content":"Hello, world!","x":0.1,"y":0.5,"fontSize":6.0,
                      "color":"#ffffff","bold":true,"background":"#00000080"}}]
}'

write_plan fade '{
  "clips":[{"start_seconds":0,"end_seconds":4,"speed":1.0,"fps_override":null}],
  "effects":[{"type":"fade","startTime":3.0,"endTime":4.0,
              "fade":{"color":"#000000","opacity":0.8}}]
}'

write_plan all_effects '{
  "clips":[{"start_seconds":0,"end_seconds":6,"speed":1.0,"fps_override":null,
            "zoom_pan":{"startRegion":{"x":0,"y":0,"width":1,"height":1},
                        "endRegion":{"x":0.2,"y":0.2,"width":0.6,"height":0.6},
                        "easing":"ease-in-out"}}],
  "effects":[
    {"type":"spotlight","startTime":1,"endTime":3,"transitionIn":0.2,"transitionOut":0.2,
     "spotlight":{"x":0.5,"y":0.5,"radius":0.2,"dimOpacity":0.6}},
    {"type":"text","startTime":1.5,"endTime":4,
     "text":{"content":"All effects active","x":0.05,"y":0.85,"fontSize":5.0,"color":"#ffffaa"}},
    {"type":"fade","startTime":5,"endTime":6,"fade":{"color":"#000000","opacity":0.7}}
  ]
}'

write_plan reverse_zoom '{
  "clips":[{"start_seconds":0,"end_seconds":5,"speed":1.0,"fps_override":null}],
  "effects":[{"type":"zoom-pan","startTime":0.5,"endTime":4.5,"reverse":true,
              "zoomPan":{"startRegion":{"x":0,"y":0,"width":1,"height":1},
                         "endRegion":{"x":0.3,"y":0.3,"width":0.4,"height":0.4},
                         "easing":"ease-in-out"}}]
}'

write_plan invalid_clip '{"clips":[{"start_seconds":-5,"end_seconds":1,"speed":1.0,"fps_override":null}]}'

# ── Runner ──────────────────────────────────────────────────────────────
RESULTS=()

run_case() {
  local name="$1"; shift
  local out="$TMPROOT/out_${name}.mp4"
  local stderr_log="$TMPROOT/${name}.stderr"
  local stdout_log="$TMPROOT/${name}.stdout"
  local time_log="$TMPROOT/${name}.time"

  echo "→ $name" >&2

  # /usr/bin/time outputs to stderr; capture separately. We point its
  # output to a file via -o so it doesn't pollute the CLI's own stderr.
  $TIME_BIN $TIME_FLAGS -o "$time_log" \
    "$CLI" "$@" --output "$out" 1>"$stdout_log" 2>"$stderr_log"
  local exit_code=$?

  local wall="?" peak_kb="?"
  if [[ "$(uname)" == "Darwin" ]]; then
    wall=$(awk '/real/ {print $1; exit}' "$time_log" 2>/dev/null || echo "?")
    peak_kb=$(awk '/maximum resident set size/ {print int($1/1024); exit}' "$time_log" 2>/dev/null || echo "?")
  else
    wall=$(awk '/Elapsed/ {print $NF; exit}' "$time_log" 2>/dev/null || echo "?")
    peak_kb=$(awk '/Maximum resident/ {print $NF; exit}' "$time_log" 2>/dev/null || echo "?")
  fi

  local size_b="-"
  local probe_codec="-" probe_w="-" probe_h="-" probe_dur="-" probe_acodec="-"
  if [[ -f "$out" ]]; then
    size_b=$(stat -f%z "$out" 2>/dev/null || stat -c%s "$out" 2>/dev/null || echo "-")
    probe=$(ffprobe -v error -show_streams -show_format -of json "$out" 2>/dev/null || echo '{}')
    probe_codec=$(echo "$probe" | jq -r '[.streams[]|select(.codec_type=="video")][0].codec_name // "-"')
    probe_w=$(echo "$probe" | jq -r '[.streams[]|select(.codec_type=="video")][0].width // "-"')
    probe_h=$(echo "$probe" | jq -r '[.streams[]|select(.codec_type=="video")][0].height // "-"')
    probe_dur=$(echo "$probe" | jq -r '.format.duration // "-"')
    probe_acodec=$(echo "$probe" | jq -r '[.streams[]|select(.codec_type=="audio")][0].codec_name // "-"')
  fi

  local env_ok=$(jq -r '.ok // empty' < "$stdout_log" 2>/dev/null)
  local env_err=$(jq -r '.error.kind // empty' < "$stdout_log" 2>/dev/null)
  local progress_lines
  if [[ -s "$stderr_log" ]]; then
    progress_lines=$(grep -c '"kind"' "$stderr_log" 2>/dev/null || true)
    progress_lines=${progress_lines//[^0-9]/}
  fi
  : "${progress_lines:=0}"

  RESULTS+=("|$name|$exit_code|${env_ok:-?}|${env_err:--}|${wall}s|${peak_kb}KB|${size_b}|${probe_codec}/${probe_acodec}|${probe_w}×${probe_h}|${probe_dur}|${progress_lines}|")
}

# Probes (no --output)
run_probe_case() {
  local name="$1"; shift
  local stderr_log="$TMPROOT/${name}.stderr"
  local stdout_log="$TMPROOT/${name}.stdout"
  local time_log="$TMPROOT/${name}.time"
  echo "→ $name" >&2
  $TIME_BIN $TIME_FLAGS -o "$time_log" "$CLI" "$@" 1>"$stdout_log" 2>"$stderr_log"
  local exit_code=$?
  local wall="?"; local peak_kb="?"
  if [[ "$(uname)" == "Darwin" ]]; then
    wall=$(awk '/real/ {print $1; exit}' "$time_log" 2>/dev/null || echo "?")
    peak_kb=$(awk '/maximum resident set size/ {print int($1/1024); exit}' "$time_log" 2>/dev/null || echo "?")
  fi
  local env_ok=$(jq -r '.ok // empty' < "$stdout_log")
  local data_summary=$(jq -c '.data | {width, height, duration_seconds, fps, codec}' < "$stdout_log" 2>/dev/null || echo '-')
  RESULTS+=("|$name|$exit_code|${env_ok:-?}|-|${wall}s|${peak_kb}KB|-|-|${data_summary}|-|0|")
}

# ── Probe ────────────────────────────────────────────────────────────────
run_probe_case probe_video_1080p probe video --input "$VID_1080P"
run_probe_case probe_video_720p probe video --input "$VID_720P"
run_probe_case probe_missing probe video --input /tmp/__nonexistent__/missing.mp4

# ── Render: trivial / fast paths ──────────────────────────────────────────
run_case render_trim_simple render apply-edits --input "$VID_1080P" --plan "$TMPROOT/plans/trim.json"
run_case render_trim_progress render --progress json apply-edits --input "$VID_1080P" --plan "$TMPROOT/plans/trim.json"

# ── Render: pipeline variations ──────────────────────────────────────────
run_case render_speed_2x render apply-edits --input "$VID_1080P" --plan "$TMPROOT/plans/speed.json"
run_case render_multiclip render apply-edits --input "$VID_1080P" --plan "$TMPROOT/plans/multiclip.json"
run_case render_freeze render apply-edits --input "$VID_1080P" --plan "$TMPROOT/plans/freeze.json"
run_case render_zoom_pan render apply-edits --input "$VID_1080P" --plan "$TMPROOT/plans/zoom_pan.json"

# ── Render: each effect ──────────────────────────────────────────────────
run_case render_spotlight render apply-edits --input "$VID_1080P" --plan "$TMPROOT/plans/spotlight.json"
run_case render_blur render apply-edits --input "$VID_1080P" --plan "$TMPROOT/plans/blur.json"
run_case render_text render apply-edits --input "$VID_1080P" --plan "$TMPROOT/plans/text.json"
run_case render_fade render apply-edits --input "$VID_1080P" --plan "$TMPROOT/plans/fade.json"
run_case render_all_effects render apply-edits --input "$VID_1080P" --plan "$TMPROOT/plans/all_effects.json"
run_case render_reverse_zoom render apply-edits --input "$VID_1080P" --plan "$TMPROOT/plans/reverse_zoom.json"

# ── Render: 720p (faster) and lower-resolution sanity ────────────────────
run_case render_720p_all render apply-edits --input "$VID_720P" --plan "$TMPROOT/plans/all_effects.json"

# ── Render: stdin plan ───────────────────────────────────────────────────
echo "→ render_stdin_plan" >&2
{ cat "$TMPROOT/plans/spotlight.json" | $TIME_BIN $TIME_FLAGS -o "$TMPROOT/render_stdin_plan.time" \
    "$CLI" render apply-edits --input "$VID_1080P" --plan - --output "$TMPROOT/out_stdin.mp4" \
    > "$TMPROOT/render_stdin_plan.stdout" 2> "$TMPROOT/render_stdin_plan.stderr"; } || true
exit_code=$?
wall=$(awk '/real/ {print $1; exit}' "$TMPROOT/render_stdin_plan.time" 2>/dev/null || echo "?")
size_b=$(stat -f%z "$TMPROOT/out_stdin.mp4" 2>/dev/null || echo "-")
env_ok=$(jq -r '.ok' < "$TMPROOT/render_stdin_plan.stdout" 2>/dev/null || echo "?")
RESULTS+=("|render_stdin_plan|$exit_code|$env_ok|-|${wall}s|-|${size_b}|stdin OK|-|-|0|")

# ── Render: error envelopes ──────────────────────────────────────────────
run_case render_invalid_input render apply-edits --input /tmp/__nonexistent__/x.mp4 --plan "$TMPROOT/plans/trim.json"
run_case render_invalid_clip render apply-edits --input "$VID_1080P" --plan "$TMPROOT/plans/invalid_clip.json"

# ── Render: extract-frame, extract-thumbnails ────────────────────────────
mkdir -p "$TMPROOT/thumbs"
echo "→ render_extract_frame" >&2
$TIME_BIN $TIME_FLAGS -o "$TMPROOT/render_extract_frame.time" \
  "$CLI" render extract-frame --input "$VID_1080P" --at 12.5 --output "$TMPROOT/frame.png" \
  >"$TMPROOT/render_extract_frame.stdout" 2>"$TMPROOT/render_extract_frame.stderr" || true
sz=$(stat -f%z "$TMPROOT/frame.png" 2>/dev/null || echo "-")
env_ok=$(jq -r '.ok' < "$TMPROOT/render_extract_frame.stdout" 2>/dev/null || echo "?")
wall=$(awk '/real/ {print $1; exit}' "$TMPROOT/render_extract_frame.time" 2>/dev/null || echo "?")
RESULTS+=("|render_extract_frame|0|$env_ok|-|${wall}s|-|${sz}|png|-|-|0|")

echo "→ render_extract_thumbnails" >&2
$TIME_BIN $TIME_FLAGS -o "$TMPROOT/render_extract_thumbnails.time" \
  "$CLI" render extract-thumbnails --input "$VID_1080P" --output-dir "$TMPROOT/thumbs" --count 8 \
  >"$TMPROOT/render_extract_thumbnails.stdout" 2>"$TMPROOT/render_extract_thumbnails.stderr" || true
n=$(jq -r '.data.paths|length' < "$TMPROOT/render_extract_thumbnails.stdout" 2>/dev/null || echo "?")
env_ok=$(jq -r '.ok' < "$TMPROOT/render_extract_thumbnails.stdout" 2>/dev/null || echo "?")
wall=$(awk '/real/ {print $1; exit}' "$TMPROOT/render_extract_thumbnails.time" 2>/dev/null || echo "?")
RESULTS+=("|render_extract_thumbnails|0|$env_ok|-|${wall}s|-|count=$n|jpg|-|-|0|")

# ── Audio: merge replace + mix ───────────────────────────────────────────
run_case render_merge_replace render merge-audio --video "$VID_1080P" --audio "$NARRATION_M4A" --replace
run_case render_merge_mix render merge-audio --video "$VID_1080P" --audio "$NARRATION_M4A"

# ── Subtitles ────────────────────────────────────────────────────────────
run_case render_burn_subs render burn-subs --input "$VID_1080P" --srt "$SRT"

# ── Summary ──────────────────────────────────────────────────────────────
{
  echo "# narrator-cli perf + behaviour suite"
  echo
  date
  echo
  echo "Source fixtures: 1080p30 30s + 720p30 15s + 20s sine narration."
  echo "Binary: \`$CLI\` (release build)."
  echo "Host: $(uname -mrs) | CPU: $(sysctl -n machdep.cpu.brand_string 2>/dev/null || cat /proc/cpuinfo|grep 'model name'|head -1|cut -d: -f2|sed 's/^ //')"
  echo
  echo "| scenario | exit | ok | err.kind | wall | peakRSS | out.size | codec(v/a) | dims | dur | progress |"
  echo "|---|---:|---|---|---:|---:|---:|---|---|---:|---:|"
  for line in "${RESULTS[@]}"; do echo "$line"; done
} > "$REPORT"

echo
echo "Wrote report: $REPORT"
echo "Inspect with: cat \"$REPORT\""
