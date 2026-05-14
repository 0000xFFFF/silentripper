import sys
import time
import threading

from globals import Variables, print_mutex, Colors
from helpers import format_time, Spinner

class StatusPrinter:

    FRAMES = Spinner.FRAMES
    BAR_WIDTH = 20

    def __init__(self, vars: Variables):
        self.total_clips = len(vars.sounded_sections)
        self.total_duration = sum(float(c["duration"]) for c in vars.sounded_sections)
        self.num_workers = max(1, int(getattr(vars.args, "threads", 1)))

        self.completed_clips = 0
        self.completed_duration = 0.0

        self.thread_slots: dict[int, int] = {}
        self.slots = [
            {
                "active": False,
                "progress": 0.0,
                "total": 0.0,
                "clip_index": None,
                "clip_total": self.total_clips,
                "frame": 0,
            }
            for _ in range(self.num_workers)
        ]

        self.start_time = 0.0
        self.lock = threading.Lock()
        self.stop_event = threading.Event()
        self.thread = threading.Thread(target=self._printer_loop, daemon=True)

    def start(self):
        self.start_time = time.time()
        with print_mutex:
            for _ in range(self.num_workers + 1):
                sys.stdout.write("\n")
            sys.stdout.flush()
        self.thread.start()

    def stop(self):
        self.stop_event.set()
        self.thread.join()

    def get_slot(self, thread_id: int) -> int:
        with self.lock:
            if thread_id in self.thread_slots:
                return self.thread_slots[thread_id]
            slot = len(self.thread_slots)
            if slot >= self.num_workers:
                slot = slot % self.num_workers
            self.thread_slots[thread_id] = slot
            return slot

    def start_clip(self, slot: int, clip_duration: float, clip_index: int, clip_total: int):
        with self.lock:
            self.slots[slot]["active"] = True
            self.slots[slot]["progress"] = 0.0
            self.slots[slot]["total"] = max(0.0, float(clip_duration))
            self.slots[slot]["clip_index"] = clip_index
            self.slots[slot]["clip_total"] = clip_total

    def update_progress(self, slot: int, seconds: float):
        with self.lock:
            total = self.slots[slot]["total"]
            self.slots[slot]["progress"] = max(0.0, min(float(seconds), total if total > 0 else 0.0))

    def finish_clip(self, slot: int, success: bool, clip_duration: float):
        with self.lock:
            self.completed_clips += 1
            self.completed_duration += float(clip_duration)
            self.slots[slot]["active"] = False
            self.slots[slot]["progress"] = 0.0
            self.slots[slot]["total"] = 0.0
            self.slots[slot]["clip_index"] = None

    def _format_bar(self, pct: float) -> str:
        pct = min(max(pct, 0.0), 1.0)
        filled = int(self.BAR_WIDTH * pct)
        bar = "=" * filled + " " * (self.BAR_WIDTH - filled)
        bar_col = f"{Colors.GREEN}{bar}{Colors.RESET}"
        pct_col = f"{Colors.YELLOW}{pct * 100:5.1f}%{Colors.RESET}"
        return f"[{bar_col}] {pct_col}"

    def _render_thread_line(self, idx: int) -> str:
        with self.lock:
            slot = dict(self.slots[idx])
        frame = f"{Colors.YELLOW}{self.FRAMES[slot['frame']]}{Colors.RESET}"
        if slot["active"] and slot["total"] > 0:
            pct = slot["progress"] / slot["total"]
            bar = self._format_bar(pct)
        else:
            bar = self._format_bar(0.0)
        clip_index = slot["clip_index"] if slot["clip_index"] is not None else "-"
        thread_label = f"{Colors.CYAN}t{idx + 1}{Colors.RESET}"
        clip_label = f"{Colors.GRAY}{clip_index}{Colors.RESET}"
        return f"{thread_label} {frame} {bar} <- {clip_label}"

    def _render_total_line(self) -> str:
        with self.lock:
            active_progress = sum(s["progress"] for s in self.slots if s["active"])
            total = self.total_duration
            completed = self.completed_duration
            completed_clips = self.completed_clips
        pct = (completed + active_progress) / total if total > 0 else 0.0
        processed_time = format_time(completed + active_progress)
        total_time = format_time(total)
        total_label = f"{Colors.CYAN}total{Colors.RESET}"
        clips_col = f"{Colors.YELLOW}{completed_clips}{Colors.RESET} / {Colors.CYAN}{self.total_clips}{Colors.RESET}"
        time_col = f"{Colors.GREEN}{processed_time}{Colors.RESET} / {Colors.CYAN}{total_time}{Colors.RESET}"
        return f"{total_label} {self._format_bar(pct)} | {clips_col} | {time_col}"

    def _printer_loop(self):
        line_count = self.num_workers + 1
        while not self.stop_event.is_set():
            with self.lock:
                for slot in self.slots:
                    slot["frame"] = (slot["frame"] + 1) % len(self.FRAMES)

            with print_mutex:
                sys.stdout.write(f"\033[{line_count}A")
                for i in range(self.num_workers):
                    sys.stdout.write("\033[2K\r" + self._render_thread_line(i) + "\n")
                sys.stdout.write("\033[2K\r" + self._render_total_line() + "\n")
                sys.stdout.flush()

            time.sleep(0.1)

        with print_mutex:
            sys.stdout.write(f"\033[{line_count}A")
            for i in range(self.num_workers):
                sys.stdout.write("\033[2K\r" + self._render_thread_line(i) + "\n")
            sys.stdout.write("\033[2K\r" + self._render_total_line() + "\n")
            sys.stdout.flush()

