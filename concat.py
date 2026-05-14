import sys
import time
import subprocess
import threading

from globals import Variables, Colors, print_mutex
from helpers import Spinner

class ConcatProgress:
    FRAMES = Spinner.FRAMES
    BAR_WIDTH = 20

    def __init__(self, total_seconds: float):
        self.total_seconds = max(0.0, float(total_seconds))
        self.progress_seconds = 0.0
        self.progress_lock = threading.Lock()
        self.stop_event = threading.Event()
        self.thread = None
        self.frame_idx = 0

    def start(self):
        self.thread = threading.Thread(target=self._spin, daemon=True)
        self.thread.start()

    def stop(self):
        self.stop_event.set()
        if self.thread:
            self.thread.join()
        with print_mutex:
            sys.stdout.write("\033[2K\r" + self._render_line() + "\n")
            sys.stdout.flush()

    def update_progress(self, seconds: float):
        with self.progress_lock:
            if self.total_seconds <= 0:
                self.progress_seconds = 0.0
            else:
                self.progress_seconds = max(0.0, min(float(seconds), self.total_seconds))

    def _format_bar(self, pct: float) -> str:
        pct = min(max(pct, 0.0), 1.0)
        filled = int(self.BAR_WIDTH * pct)
        bar = "=" * filled + " " * (self.BAR_WIDTH - filled)
        bar_col = f"{Colors.GREEN}{bar}{Colors.RESET}"
        pct_col = f"{Colors.YELLOW}{pct * 100:5.1f}%{Colors.RESET}"
        return f"[{bar_col}] {pct_col}"

    def _render_line(self) -> str:
        with self.progress_lock:
            progress = self.progress_seconds
        pct = progress / self.total_seconds if self.total_seconds > 0 else 0.0
        frame = f"{Colors.YELLOW}{self.FRAMES[self.frame_idx]}{Colors.RESET}"
        label = f"{Colors.CYAN}concat{Colors.RESET}"
        return f"{label} {frame} {self._format_bar(pct)}"

    def _spin(self):
        while not self.stop_event.is_set():
            with print_mutex:
                sys.stdout.write("\033[2K\r" + self._render_line())
                sys.stdout.flush()
            self.frame_idx = (self.frame_idx + 1) % len(self.FRAMES)
            time.sleep(0.1)

def concat_prepare_list(vars: Variables):
    with open("clip_list.txt", "w") as clip_list_file:
        for clip in vars.sounded_sections:
            if clip["success"]:
                clip_list_file.write(f"file '{clip['file']}'\n")

def concat(vars: Variables):

    assert vars.args is not None

    concat_cmd = [
        'ffmpeg', '-hide_banner', '-loglevel', 'error',
        '-progress', 'pipe:1', '-nostats',
        '-f', 'concat', '-safe', '0',
        '-y', '-nostdin'
    ]

    if vars.args.gpu:
        concat_cmd += ['-hwaccel', 'vaapi', '-vaapi_device', '/dev/dri/renderD128']

    concat_cmd += ['-i', 'clip_list.txt']

    # Output options (after -i)
    if vars.args.gpu:
        concat_cmd += ['-vf', 'format=nv12,hwupload', '-c:v', 'h264_vaapi']
    elif vars.args.copy:
        concat_cmd += ['-c', 'copy']

    concat_cmd += [vars.output_file_name]

    sys.stdout.write(f"Concatenating clips into: {vars.output_file_name}\n")
    sys.stdout.flush()

    total_duration = vars.total_sounded_duration or sum(float(c["duration"]) for c in vars.sounded_sections)
    progress = ConcatProgress(total_duration)
    progress.start()
    result = _run_ffmpeg_with_progress(concat_cmd, progress.update_progress)
    progress.update_progress(total_duration)
    progress.stop()
    if result.returncode != 0:
        error_text = result.stderr
        if isinstance(error_text, bytes):
            error_text = error_text.decode("utf-8", errors="replace")
        sys.stdout.write(f"Concatenation failed with error: {error_text}\n")
        sys.stdout.flush()
    else:
        sys.stdout.write("Concatenation succeeded!\n")
        sys.stdout.flush()

def _parse_ffmpeg_time(time_text: str) -> float:
    if not time_text or time_text == "N/A":
        return 0.0
    parts = time_text.split(":")
    if len(parts) != 3:
        return 0.0
    hours, minutes, seconds = parts
    try:
        return (int(hours) * 3600) + (int(minutes) * 60) + float(seconds)
    except ValueError:
        return 0.0

def _run_ffmpeg_with_progress(ffmpeg_cmd, progress_cb):
    proc = subprocess.Popen(
        ffmpeg_cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1
    )

    stderr_output = ""
    if proc.stdout is not None:
        for line in proc.stdout:
            line = line.strip()
            if not line:
                continue
            if line.startswith("out_time_ms="):
                value = line.split("=", 1)[1]
                try:
                    progress_cb(int(value) / 1_000_000.0)
                except ValueError:
                    pass
            elif line.startswith("out_time="):
                progress_cb(_parse_ffmpeg_time(line.split("=", 1)[1]))
            elif line == "progress=end":
                progress_cb(float("inf"))

    if proc.stderr is not None:
        stderr_output = proc.stderr.read()

    return subprocess.CompletedProcess(
        args=ffmpeg_cmd,
        returncode=proc.wait(),
        stdout="",
        stderr=stderr_output
    )
