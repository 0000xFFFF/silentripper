import sys
import time
import subprocess
import threading
import concurrent.futures

from globals import Variables, print_mutex, Colors
from status_thread import StatusPrinter
from helpers import format_time, format_elapsed, Spinner

def start_work(vars: Variables, status: StatusPrinter):

    assert vars.args is not None

    # Track line numbers for each clip (in verbose mode)
    line_counter = [0]  # Use list for nonlocal in nested function

    # Function to process a single clip
    def process_clip(clip, i, total):

        assert vars.args is not None
        ffmpeg_cmd = ['ffmpeg', '-y', '-nostdin']

        # Input options (before -i)
        if vars.args.gpu:
            ffmpeg_cmd += ['-hwaccel', 'vaapi', '-vaapi_device', '/dev/dri/renderD128']

        # Input file
        ffmpeg_cmd += ['-ss', clip["start"], '-to', clip["end"], '-i', str(vars.input_full_file_path)]

        if not vars.args.gpu and vars.args.copy:
            ffmpeg_cmd += ['-c', 'copy']

        # Output options (after -i)
        if vars.args.gpu:
            ffmpeg_cmd += ['-vf', 'format=nv12,hwupload', '-c:v', 'h264_vaapi']

        ffmpeg_cmd += [clip["file"]]

        my_line = None
        spinner = None
        if vars.args.verbose:
            clip_info = f"{i}/{total} {clip['file']}: {format_time(clip['start'])} -- {format_time(clip['end'])} ({format_time(clip['duration'])}) "
            with print_mutex:
                my_line = line_counter[0]
                line_counter[0] += 1
                sys.stdout.write(clip_info + "\n")
                sys.stdout.flush()
            
            # Start spinner for this line - pass line_counter and print_mutex
            spinner = Spinner(clip_info, my_line, line_counter, print_mutex)
            spinner.start()

        if vars.args.very_verbose:
            sys.stdout.write(f"{' '.join(ffmpeg_cmd)}")
            sys.stdout.flush()

        clip_start = time.time()
        result = None
        if vars.args.very_verbose:
            result = subprocess.run(ffmpeg_cmd)
        else:
            result = subprocess.run(ffmpeg_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        elapsed = time.time() - clip_start

        # Stop spinner before printing result
        if spinner:
            spinner.stop()

        success = result.returncode == 0

        if vars.args.verbose and my_line is not None:
            with print_mutex:
                status_word = f"{Colors.GREEN}ok{Colors.RESET}" if success else f"{Colors.RED}fail{Colors.RESET}"
                lines_down = line_counter[0] - my_line
                escape_sequence = ""
                if lines_down > 0:
                    escape_sequence += f"\033[{lines_down}A"
                escape_sequence += "\033[2K\r"
                escape_sequence += f"{clip_info}{status_word} (took: {format_elapsed(elapsed)})\n"
                if lines_down > 0:
                    escape_sequence += f"\033[{lines_down}B"
                sys.stdout.write(escape_sequence)
                sys.stdout.flush()
        elif not vars.args.verbose:
            status.update(success, float(clip["duration"]))

        return clip["file"], success

    num_workers = vars.args.threads
    lock = threading.Lock()
    
    failed_conversions = 1

    with concurrent.futures.ThreadPoolExecutor(max_workers=num_workers) as executor:
        total = len(vars.sounded_sections)
        futures = {executor.submit(process_clip, clip, i, total): clip for i, clip in enumerate(vars.sounded_sections, start=1)}
        for future in concurrent.futures.as_completed(futures):
            output_file, success = future.result()
            with lock:
                for c in vars.sounded_sections:
                    if c["file"] == output_file:
                        c["success"] = success
                        if not success:
                            failed_conversions += 1
                        break

    return failed_conversions
