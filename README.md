# silentripper

[![Python 3.12.5](https://img.shields.io/badge/Python-3.12.5-yellow.svg)](http://www.python.org/download/)

Remove silent parts from video using ffmpeg.

## Requirements
* python
* ffmpeg (+ffprobe)

## Running
```sh
./silentripper <video.mp4>
```

# Running - faster cutting
```sh
./silentripper <video.mp4> -m 1 -d 1 -c
```
Set the minimum sounded and muted duration to 1 to avoid glitchy output caused by the "-c" (copy codec) option.

## Usage
```
usage: silentripper [-h] [-d sec] [-m sec] [-n dB] [-c] [-p] filename

Remove silent parts from video using FFmpeg

positional arguments:
  filename

options:
  -h, --help            show this help message and exit
  -d sec, --duration sec
                        Silence duration in seconds (default: 1, min: 1)
  -m sec, --min_duration sec
                        Minimum duration for each sounded clip in seconds (default: 0)
  -n dB, --noise dB     Noise level in dB (default: -30)
  -c, --copy            Use copy codec for faster but potentially glitchy output (increase -m option for less glitchy output)
  -p, --pause           Prompt before each action
```
