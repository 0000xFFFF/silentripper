#!/usr/bin/env python3
import sys
import os
import subprocess
import time
import argparse
from pathlib import Path

# Argument parsing
parser = argparse.ArgumentParser(description='Remove silent parts from video using FFmpeg')
parser.add_argument('-d', '--duration', metavar='sec', type=float, default=1, help="Silence duration in seconds (default: 1, min: 1)")
parser.add_argument('-m', '--min_duration', metavar='sec', type=float, default=0, help="Minimum duration for each sounded clip in seconds (default: 0)")
parser.add_argument('-n', '--noise', metavar='dB', type=int, default=-30, help="Noise level in dB (default: -30)")
parser.add_argument('-c', '--copy', action='store_true', help="Use copy codec for faster but potentially glitchy output (increase -m option for less glitchy output))")
parser.add_argument('-p', '--pause', action='store_true', help="Prompt before each action")
parser.add_argument('filename', type=argparse.FileType('r'))
args = parser.parse_args()

# File details
input_file_path = Path(args.filename.name)
file_name = input_file_path.stem
file_extension = input_file_path.suffix
full_file_path = input_file_path.resolve().as_posix()

# Global variables
altered_clips = 0
total_muted_duration = 0
total_sounded_duration = 0
failed_conversions = 0

# Ensure silence duration is at least 1 second
#args.duration = max(float(args.duration), 1)

# Get total video duration
def get_video_duration(file_path):
    ffprobe_cmd = [
        'ffprobe', '-v', 'error', '-show_entries', 'format=duration',
        '-of', 'default=noprint_wrappers=1:nokey=1', file_path
    ]
    result = subprocess.run(ffprobe_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return float(result.stdout.strip())

total_duration = get_video_duration(full_file_path)

# Print initial settings
print(f"Noise level.............: {args.noise} dB")
print(f"Silence duration........: {args.duration} s")
print(f"Minimum sounded duration: {args.min_duration} s")
print(f"Copy codec..............: {args.copy}")
print(f"Total video duration....: {total_duration:.2f} s")

# Run FFmpeg to detect silent sections
ffmpeg_proc = subprocess.Popen(
    [
        'ffmpeg', '-hide_banner', '-vn', '-i', full_file_path,
        '-af', f'silencedetect=n={args.noise}dB:d={args.duration}', '-f', 'null', '-'
    ],
    stdout=subprocess.PIPE, stderr=subprocess.STDOUT
)

# Parse FFmpeg output for silence start/end timestamps
muted_sections = []
for line in ffmpeg_proc.stdout.read().decode().split("\n"):
    line = line.strip()
    if "silencedetect" not in line:
        continue
    if "silence_start: " in line:
        start_time = line.split("silence_start: ")[1]
    elif "silence_end: " in line:
        end_time, duration = line.split("silence_end: ")[1].split(" | silence_duration: ")
        muted_sections.append([start_time, end_time, duration])

# Print muted sections
print("===[ MUTED TIMESTAMPS (START, END, DURATION) ]===")
for idx, (start, end, duration) in enumerate(muted_sections, start=1):
    total_muted_duration += float(duration)
    print(f"{idx}. Start: {start}, End: {end}, Duration: {duration} seconds")

# Create subclips for sounded sections
sounded_clips = []

def add_sounded_clip(start, end):
    global altered_clips
    output_file = f"clip_{len(sounded_clips)+1}.mts"

    if end is None:  # If no next mute section, use the total duration of the video
        end = str(total_duration)

    duration = round(float(end) - float(start), 5)

    if duration < args.min_duration:
        altered_clips += 1
        end = str(round(float(end) + args.min_duration - duration, 5))
        duration = round(float(end) - float(start), 5)

    sounded_clips.append([start, end, duration, output_file])

# Add clips for sounded parts between muted sections
if muted_sections:
    # Handle the first sounded section (before the first silence)
    first_mute_start = muted_sections[0][0]
    add_sounded_clip("0", first_mute_start)

    # Handle the sounded sections between silences
    for i in range(len(muted_sections)):
        start = muted_sections[i][1]
        end = muted_sections[i+1][0] if i+1 < len(muted_sections) else None
        add_sounded_clip(start, end)

# Print sounded sections
print("===[ SOUNDED TIMESTAMPS (OUTPUT FILE, START, END, DURATION) ]===")
for idx, (start, end, duration, output_file) in enumerate(sounded_clips, start=1):
    total_sounded_duration += float(duration)
    print(f"{idx}. Output: {output_file}, Start: {start}, End: {end}, Duration: {duration} seconds")

# Convert seconds to HH:MM:SS format
def format_time(seconds):
    return time.strftime("%H:%M:%S", time.gmtime(seconds))

# Summary
print(f"\nMuted sections: {len(muted_sections)}")
print(f"Sounded sections: {len(sounded_clips)}")
print(f"Altered clips: {altered_clips}")
print(f"Total muted duration: {format_time(total_muted_duration)} ({total_muted_duration:.2f} seconds)")
print(f"Total sounded duration: {format_time(total_sounded_duration)} ({total_sounded_duration:.2f} seconds)\n")

# Pause if necessary
if args.pause:
    input("Press Enter to start processing...")

# Create a list of clips to concatenate
with open("clip_list.txt", "w") as clip_list_file:
    l = len(sounded_clips)
    for i, (start, end, duration, output_file) in enumerate(sounded_clips):
        ffmpeg_cmd = [
            'ffmpeg', '-hide_banner', '-v', 'quiet',
            '-y', '-i', full_file_path,
            '-ss', start, '-to', end
        ]
        if args.copy:
            ffmpeg_cmd += ['-c', 'copy']

        ffmpeg_cmd += [output_file]

        print(f"{i+1}/{l}: {start} -- {end} ({duration}) --> {output_file} -- ...", end='')

        # Run the ffmpeg command with subprocess.run() and wait for it to complete
        result = subprocess.run(ffmpeg_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

        if result.returncode == 0:
            clip_list_file.write(f"file '{output_file}'\n")
            clip_list_file.flush()  # Explicit flush after every write
            print("\b\b\bsuccess")
        else:
            failed_conversions += 1
            print("\b\b\bfailed")

print(f"\nFailed conversions: {failed_conversions}")

# Concatenate the sounded clips into the final output file
print("===[ CONCAT ]===")
concat_cmd = [
    'ffmpeg', '-hide_banner', '-loglevel', 'error', '-f', 'concat',
    '-y', '-i', 'clip_list.txt'
]

if args.copy:
    concat_cmd += ['-c', 'copy']

output_file_name = f"{file_name}_cut{file_extension}"
concat_cmd += [output_file_name]

print(f"\nConcatenating clips into: {output_file_name}")
result = subprocess.run(concat_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
if result.returncode != 0:
    print(f"Concatenation failed with error: {result.stderr.decode('utf-8')}")
else:
    print(f"Concatenation succeeded!")

# Pause if necessary before cleanup
if args.pause:
    input("Press Enter to clean up temporary files...")

# Clean up temporary files
for _, _, _, output_file in sounded_clips:
    os.remove(output_file)
os.remove("clip_list.txt")

print("Cleanup complete.")

