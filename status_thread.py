import sys
import time
import threading
from globals import Colors, Variables, print_mutex
from helpers import format_time

class StatusPrinter:

    def __init__(self, vars: Variables):
        self.total_clips = len(vars.sounded_sections)
        self.total_duration = vars.total_duration

        self.ok = 0
        self.fail = 0
        self.processed_duration = 0.0
        self.start_time = 0.0
        self.lock = threading.Lock()
        self.stop_event = threading.Event()
        self.thread = threading.Thread(target=self._printer_loop, daemon=True)

    def start(self):
        self.start_time = time.time()
        self.thread.start()

    def stop(self):
        self.stop_event.set()
        self.thread.join()

    def update(self, success: bool, clip_duration: float):
        with self.lock:
            if success:
                self.ok += 1
            else:
                self.fail += 1

            self.processed_duration += clip_duration

    def _render_line(self) -> str:
        with self.lock:
            ok = self.ok
            fail = self.fail
            dur = self.processed_duration

        processed = ok + fail
        elapsed = time.time() - self.start_time
        cps = processed / elapsed if elapsed > 0 else 0.0

        ok_col = (f"{Colors.GREEN}" f"{Colors.BOLD}" f"{ok}" f"{Colors.RESET}")
        total_col = (f"{Colors.CYAN}" f"{self.total_clips}" f"{Colors.RESET}")
        fail_col = (f"{Colors.RED}{fail}{Colors.RESET}" if fail else f"{Colors.GRAY}0{Colors.RESET}")
        dur_col = (f"{Colors.GREEN}" f"{format_time(dur)}" f"{Colors.RESET}")
        total_dur_col = (f"{Colors.CYAN}" f"{format_time(self.total_duration)}" f"{Colors.RESET}")
        cps_col = (f"{Colors.YELLOW}" f"{cps:.2f} CpS" f"{Colors.RESET}")
        return (f"\r" f"  {ok_col} / {total_col}" f" | {fail_col}" f" | {dur_col} / {total_dur_col}" f" | {cps_col}   ")

    def _printer_loop(self):
        while not self.stop_event.is_set():
            with print_mutex:
                sys.stdout.write(self._render_line())
                sys.stdout.flush()
            time.sleep(0.1)
        with print_mutex:
            sys.stdout.write(self._render_line())
            sys.stdout.write("\n")
            sys.stdout.flush()

