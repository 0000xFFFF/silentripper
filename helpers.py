import os
import sys
import time
import subprocess
from globals import Variables

def format_time(seconds):
    return time.strftime("%H:%M:%S", time.gmtime(float(seconds)))

# Get total video duration
def get_video_duration(file_path):
    ffprobe_cmd = [
        'ffprobe', '-v', 'error', '-show_entries', 'format=duration',
        '-of', 'default=noprint_wrappers=1:nokey=1', file_path
    ]
    result = subprocess.run(ffprobe_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return float(result.stdout.strip())


def pause(do_pause, message):
    if do_pause:
        try:
            input(message)
        except:
            exit()


def cleanup(vars: Variables):
    for clip in vars.sounded_sections:
        try:
            os.remove(clip["file"])
        except:
            pass
    try:
        os.remove("clip_list.txt")
    except:
        pass

    sys.stdout.write("Cleanup complete.\n")


def print_video_info(vars: Variables):
    sys.stdout.write(f"Total video duration....: {format_time(vars.total_duration)} ({vars.total_duration:.2f} s)\n")
    sys.stdout.write(f"Input file..............: {vars.input_file_path}\n")
    sys.stdout.write(f"Output file.............: {vars.output_file_name}\n")
    sys.stdout.flush()
