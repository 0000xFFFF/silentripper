import sys
from globals import Variables
from helpers import format_time
import subprocess

def get_muted_sections(vars: Variables):

    assert vars.args is not None

    # Run FFmpeg to detect silent sections
    ffmpeg_proc = subprocess.Popen(
        [
            'ffmpeg', '-nostdin', '-hide_banner', '-vn', '-i', str(vars.input_full_file_path),
            '-af', f'silencedetect=n={vars.args.noise}dB:d={vars.args.duration}', '-f', 'null', '-'
        ],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT
    )

    # Parse FFmpeg output for silence start/end timestamps
    start_time = None
    if ffmpeg_proc.stdout is not None:
        output = ffmpeg_proc.stdout.read().decode()

        for line in output.splitlines():
            line = line.strip()

            if "silencedetect" not in line:
                continue

            if "silence_start: " in line:
                start_time = line.split("silence_start: ")[1]

            elif "silence_end: " in line and start_time is not None:
                end_time, duration = line.split(
                    "silence_end: "
                )[1].split(" | silence_duration: ")

                vars.muted_sections.append([start_time, end_time, duration])

                start_time = None

    # Process muted sections
    if vars.args.verbose:
        sys.stdout.write("===[ MUTED TIMESTAMPS (START, END, DURATION) ]===\n")

    for idx, (start, end, duration) in enumerate(vars.muted_sections, start=1):
        vars.total_muted_duration += float(duration)
        if vars.args.verbose:
            sys.stdout.write(f"{idx}. Start: {format_time(start)}, End: {format_time(end)}, Duration: {format_time(duration)} seconds\n")
    if vars.args.verbose:
        sys.stdout.flush()


def get_sounded_sections(vars: Variables):


    assert vars.args is not None

    altered_clips = 0

    def add_sounded_clip(start, end):

        assert vars.args is not None

        nonlocal altered_clips
        output_file = f"clip_{len(vars.sounded_sections)+1}.mp4"

        if end is None:
            end = str(vars.total_duration)

        duration = round(float(end) - float(start), 5)
        if duration < vars.args.min_duration:
            altered_clips += 1
            end = str(round(float(end) + vars.args.min_duration - duration, 5))
            duration = round(float(end) - float(start), 5)

        vars.sounded_sections.append({
            "start": start,
            "end": end,
            "duration": duration,
            "file": output_file,
            "success": False
        })

    # Add clips for sounded parts
    if vars.muted_sections:
        # Before first silence
        first_mute_start = vars.muted_sections[0][0]
        add_sounded_clip("0", first_mute_start)

        # Between silences
        for i in range(len(vars.muted_sections)):
            start = vars.muted_sections[i][1]
            end = vars.muted_sections[i+1][0] if i+1 < len(vars.muted_sections) else None
            add_sounded_clip(start, end)

    if vars.args.verbose:
        sys.stdout.write("===[ SOUNDED TIMESTAMPS (OUTPUT FILE, START, END, DURATION) ]===\n")

    for idx, clip in enumerate(vars.sounded_sections, start=1):
        vars.total_sounded_duration += float(clip["duration"])
        if vars.args.verbose:
            sys.stdout.write(f"{idx}. Output: {clip['file']}, Start: {format_time(clip['start'])}, End: {format_time(clip['end'])}, Duration: {format_time(clip['duration'])} seconds\n")
    if vars.args.verbose:
        sys.stdout.flush()



def audio_summary(vars: Variables):
    sys.stdout.write(f"Muted sections: {len(vars.muted_sections)}\n")
    sys.stdout.write(f"Sounded sections: {len(vars.sounded_sections)}\n")
    sys.stdout.write(f"Altered clips: {vars.altered_clips}\n")
    sys.stdout.write(f"Total muted duration: {format_time(vars.total_muted_duration)} ({vars.total_muted_duration:.2f} seconds)\n")
    sys.stdout.write(f"Total sounded duration: {format_time(vars.total_sounded_duration)} ({vars.total_sounded_duration:.2f} seconds)\n")
    sys.stdout.flush()

    if vars.total_muted_duration == 0:
        sys.stdout.write(f"No muted duration. Ajust --noise and try again.\n")
        sys.stdout.flush()
        exit()
