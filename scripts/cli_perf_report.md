# narrator-cli perf + behaviour suite

Sun Apr 19 17:12:41 EEST 2026

Source fixtures: 1080p30 30s + 720p30 15s + 20s sine narration.
Binary: `/Users/alexandruroman/VariousProjects/VideoNarator/src-tauri/target/release/narrator-cli` (release build).
Host: Darwin 25.4.0 arm64 | CPU: Apple M3 Pro

| scenario | exit | ok | err.kind | wall | peakRSS | out.size | codec(v/a) | dims | dur | progress |
|---|---:|---|---|---:|---:|---:|---|---|---:|---:|
|probe_video_1080p|0|true|-|0.05s|17272KB|-|-|{"width":1920,"height":1080,"duration_seconds":30.0,"fps":30.0,"codec":"h264"}|-|0|
|probe_video_720p|0|true|-|0.05s|14456KB|-|-|{"width":1280,"height":720,"duration_seconds":15.0,"fps":30.0,"codec":"h264"}|-|0|
|probe_missing|1|?|-|0.04s|10880KB|-|-|{"width":null,"height":null,"duration_seconds":null,"fps":null,"codec":null}|-|0|
|render_trim_simple|0|true|-|0.09s|18224KB|395254|h264/aac|1920×1080|5.015510|0|
|render_trim_progress|0|true|-|0.09s|18188KB|395254|h264/aac|1920×1080|5.015510|1|
|render_speed_2x|0|true|-|2.59s|736872KB|2172888|h264/aac|1920×1080|5.007000|0|
|render_multiclip|0|true|-|3.03s|573860KB|6958314|h264/aac|1920×1080|13.339000|0|
|render_freeze|0|true|-|1.80s|477708KB|3838066|h264/aac|1920×1080|8.000000|0|
|render_zoom_pan|0|true|-|3.25s|501036KB|4836286|h264/aac|1920×1080|5.000000|0|
|render_spotlight|0|true|-|2.14s|511152KB|2974194|h264/aac|1920×1080|5.000000|0|
|render_blur|0|true|-|4.64s|508872KB|2398971|h264/aac|1920×1080|4.000000|0|
|render_text|0|true|-|0.99s|485836KB|2190143|h264/aac|1920×1080|4.000000|0|
|render_fade|0|true|-|1.03s|462376KB|2082821|h264/aac|1920×1080|4.000000|0|
|render_all_effects|0|true|-|4.66s|541220KB|5480311|h264/aac|1920×1080|6.000000|0|
|render_reverse_zoom|0|true|-|2.77s|517380KB|4114377|h264/aac|1920×1080|5.000000|0|
|render_720p_all|0|true|-|2.18s|223340KB|3619812|h264/aac|1280×720|6.000000|0|
|render_stdin_plan|0|true|-|2.13s|-|2974194|stdin OK|-|-|0|
|render_invalid_input|1|?|VideoProbeError|0.04s|10944KB|-|-/-|-×-|-|0|
|render_invalid_clip|1|?|ExportError|0.05s|17300KB|-|-/-|-×-|-|0|
|render_extract_frame|0|true|-|0.12s|-|53013|png|-|-|0|
|render_extract_thumbnails|0|true|-|0.24s|-|count=8|jpg|-|-|0|
|render_merge_replace|0|true|-|0.23s|21592KB|2211343|h264/aac|1920×1080|30.000000|0|
|render_merge_mix|0|true|-|0.71s|47056KB|2946508|h264/aac|1920×1080|30.000000|0|
|render_burn_subs|0|true|-|1.45s|257300KB|11932607|h264/aac|1920×1080|30.000000|0|
