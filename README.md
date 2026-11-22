# silentripper

Remove silent parts from video. Rust rewrite of 0000xFFFF/silentripper using libav directly
(via `ffmpeg-next`) - **no ffmpeg/ffprobe subprocesses**.

- Silence detection is done in-process (native port of FFmpeg's `silencedetect`; boundaries match to ~0.05 ms).
- Decode / cut / encode / mux all happen inside libav.
- The video decoder skips silent stretches entirely (only decodes from the keyframe before each kept clip).
- Audio is cut sample-accurately; video encoded with libx264/libx265 or a GPU encoder.

## Build
Needs FFmpeg dev libs (libavcodec/format/util/swresample/swscale/avfilter/avdevice, pkg-config, clang).

    sudo apt install libavcodec-dev libavformat-dev libavutil-dev libswresample-dev libswscale-dev \
                     libavfilter-dev libavdevice-dev pkg-config clang
    cargo build --release        # binary: target/release/silentripper

## Experimenting with the encoder
Use `--limit` to encode only the first N seconds of output, so each experiment takes seconds, and look at the
`stage busy time` block at the end to see where the time went. Compare variants in a loop:

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

Discovering options: `silentripper --list-encoders` shows what your libav has, `silentripper --list-opts h264_vaapi`
lists every option an encoder accepts (use any of them as `--enc-opt NAME=VALUE`, repeatable, applied last).
Unknown option names are rejected instead of silently ignored. `--dry-run` only prints the cut plan,
`--log-level info|debug` shows libav's own messages. The applied encoder options are printed before encoding.

## Usage
    silentripper video.mp4                       # -> video_ripped.mp4
    silentripper video.mp4 -c                    # stream copy, near-instant, cuts on keyframes
    silentripper video.mp4 --gpu                 # auto-detect GPU
    silentripper video.mp4 --gpu-nvidia          # NVENC
    silentripper video.mp4 --gpu-amd             # VAAPI (Linux) / AMF (Windows)

Same flags as the original: `-d sec` (min silence, default 1), `-m sec` (min kept clip), `-n dB` (default -30),
`-c` copy, `-p` pause. Extras: `-o`, `--pad sec`, `--codec h264|hevc`, `--crf N`, `--preset`, `--abitrate kbps`.
`SILENTRIPPER_LOG=1` shows libav's own log output.

## GPU notes
- `--gpu` picks the vendor from PCI ids (Linux) or available encoders, tries the hardware encoder and falls back to CPU if it can't open.
- `--gpu-nvidia` / `--gpu-amd` never fall back: you get libav's error instead.
- Only *encoding* is on the GPU; decoding stays on the CPU.
- AMD on Linux needs Mesa VAAPI (`/dev/dri/renderD*`); on Windows it uses AMF.
