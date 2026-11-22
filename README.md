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
