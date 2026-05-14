import os
import sys
import time
import subprocess
from globals import Variables

def format_time(seconds):
    return time.strftime("%H:%M:%S", time.gmtime(float(seconds)))

def format_elapsed(elapsed: float) -> str:
    """Convert elapsed time in seconds to human readable format"""
    if elapsed < 0.001:
        return f"{elapsed*1000:.2f}ms"
    elif elapsed < 1:
        return f"{elapsed*1000:.0f}ms"
    elif elapsed < 60:
        return f"{elapsed:.1f}s"
    elif elapsed < 3600:
        minutes = int(elapsed // 60)
        seconds = int(elapsed % 60)
        return f"{minutes}m {seconds}s"
    else:
        hours = int(elapsed // 3600)
        minutes = int((elapsed % 3600) // 60)
        seconds = int(elapsed % 60)
        return f"{hours}h {minutes}m {seconds}s"

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
