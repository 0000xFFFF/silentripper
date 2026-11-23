## Experimenting with the encoder
Use `--limit` to encode only the first N seconds of output, so each experiment takes seconds, and look at the
`stage busy time` block at the end to see where the time went. Compare variants in a loop:

```sh
# baseline: 2 minutes of output on the AMD GPU
silentripper video.mkv --gpu-amd --limit 120 -o /tmp/t.mkv

# how deep should the VAAPI encoder pipeline be? (libav default is 2)
for d in 2 4 8 16 32; do
  echo "== async_depth=$d"
  silentripper video.mkv --gpu-amd --limit 120 --async-depth $d -o /tmp/t.mkv 2>&1 | grep -E "wall clock|video encode"
done

# trade quality for speed in the VAAPI driver (higher = faster)
silentripper video.mkv --gpu-amd --limit 120 --vaapi-quality 7 -o /tmp/t.mkv

# other VAAPI knobs: rate control mode, B-frames, low-power engine
silentripper video.mkv --gpu-amd --limit 120 --vaapi-rc CQP --crf 26 -o /tmp/t.mkv
silentripper video.mkv --gpu-amd --limit 120 --bframes 2 -o /tmp/t.mkv
silentripper video.mkv --gpu-amd --limit 120 --enc-opt low_power=1 -o /tmp/t.mkv

# bitrate instead of constant quality, HEVC, bigger frame queue between threads
silentripper video.mkv --gpu-amd --limit 120 --bitrate 6000 --vaapi-rc VBR -o /tmp/t.mkv
silentripper video.mkv --gpu-amd --limit 120 --codec hevc -o /tmp/t.mkv
silentripper video.mkv --gpu-amd --limit 120 --queue 16 -o /tmp/t.mkv

# CPU for comparison
silentripper video.mkv --limit 120 --preset ultrafast -o /tmp/t.mkv
silentripper video.mkv --limit 120 --preset medium --threads 12 -o /tmp/t.mkv

# NVIDIA
silentripper video.mkv --gpu-nvidia --limit 120 --nvenc-preset p1 -o /tmp/t.mkv
```

Discovering options: `silentripper --list-encoders` shows what your libav has, `silentripper --list-opts h264_vaapi`
lists every option an encoder accepts (use any of them as `--enc-opt NAME=VALUE`, repeatable, applied last).
Unknown option names are rejected instead of silently ignored. `--dry-run` only prints the cut plan,
`--log-level info|debug` shows libav's own messages. The applied encoder options are printed before encoding.
