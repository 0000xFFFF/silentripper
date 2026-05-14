from argparse import Namespace
from typing import Optional
import threading
from enum import StrEnum
from pathlib import PurePath

VERSION = "0.8.1"

class Colors(StrEnum):
    GREEN   = "\033[32m"
    RED     = "\033[31m"
    GRAY    = "\033[90m"
    CYAN    = "\033[36m"
    YELLOW  = "\033[33m"
    RESET   = "\033[0m"
    BOLD    = "\033[1m"

print_mutex = threading.Lock()

class Variables:
    args: Optional[Namespace] = None

    program_start_time: float = 0.0
    altered_clips: int = 0
    total_muted_duration: float = 0.0
    total_sounded_duration: float = 0.0
    failed_conversions: int = 0
    total_duration: float = 0.0

    input_file_path: PurePath = PurePath()
    input_file_name: str = ""
    input_file_extension: str = ""
    input_full_file_path: PurePath = PurePath()
    output_file_path: str = ""
    output_file_name: str = ""

    temp_extension: str = ""

    muted_sections = []
    sounded_sections = []

