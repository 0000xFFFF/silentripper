import os
import sys
import time
import subprocess
import threading
from globals import Variables, print_mutex

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


def hide_cursor():
    sys.stdout.write("\033[?25l")
    sys.stdout.flush()


def show_cursor():
    sys.stdout.write("\033[?25h")
    sys.stdout.flush()


class Spinner:
    """Thread-safe spinner for displaying progress while processing"""
    FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
    
    def __init__(self, line_info: str, line_number: int, line_counter, print_lock):
        """
        Initialize spinner
        line_info: the clip info line to display with spinner
        line_number: which line this was printed on
        line_counter: shared list [current_line_count]
        print_lock: the print_mutex
        """
        self.line_info = line_info
        self.line_number = line_number
        self.line_counter = line_counter
        self.print_lock = print_lock
        self.stop_event = threading.Event()
        self.thread = None
        self.frame_idx = 0
    
    def start(self):
        """Start the spinner in a background thread"""
        self.thread = threading.Thread(target=self._spin, daemon=True)
        self.thread.start()
    
    def stop(self):
        """Stop the spinner"""
        self.stop_event.set()
        if self.thread:
            self.thread.join()
    
    def _spin(self):
        """Spinner loop - updates the line with rotating frames"""
        while not self.stop_event.is_set():
            with self.print_lock:
                # Calculate how many lines down from our line the cursor currently is
                lines_down = self.line_counter[0] - self.line_number
                escape_sequence = ""
                if lines_down > 0:
                    escape_sequence += f"\033[{lines_down}A"
                escape_sequence += f"\033[2K\r{self.line_info}{self.FRAMES[self.frame_idx]}"
                if lines_down > 0:
                    escape_sequence += f"\033[{lines_down}B"
                sys.stdout.write(escape_sequence)
                sys.stdout.flush()
            
            self.frame_idx = (self.frame_idx + 1) % len(self.FRAMES)
            time.sleep(0.1)

