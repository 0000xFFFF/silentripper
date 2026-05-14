import sys
import argparse
from globals import VERSION, Variables
from helpers import format_time

def get_arguments():
    parser = argparse.ArgumentParser(description='Remove silent parts from video using FFmpeg')
    parser.add_argument('-d', '--duration', metavar='sec', type=float, default=1, help="Silence duration in seconds (default: 1, min: 1)")
    parser.add_argument('-m', '--min_duration', metavar='sec', type=float, default=1, help="Minimum duration for each sounded clip in seconds (default: 1)")
    parser.add_argument('-n', '--noise', metavar='dB', type=int, default=-40, help="Noise level in dB (default: -40)")
    parser.add_argument('-c', '--copy', action='store_true', help="Use copy codec for faster but potentially glitchy output")
    parser.add_argument('-p', '--pause', action='store_true', help="Prompt before each action")
    parser.add_argument('-t', '--threads', metavar='N', type=int, default=4, help="Number of worker threads")
    parser.add_argument('-o', '--output', metavar='file', type=str, help="Output filename (default: \"filename} (cut).{ext}\")") 
    parser.add_argument('-g', '--gpu', action='store_true', help="Tell FFmpeg to use GPU (AMD is by default edit the script to use something else)")
    parser.add_argument('-v', '--verbose', action='store_true', help="Be verbose")
    parser.add_argument('-vv', '--very_verbose', action='store_true', help="Be very verbose")
    parser.add_argument('--version', action='version', version=f'%(prog)s {VERSION}')
    parser.add_argument('filename', type=str)
    args = parser.parse_args()
    
    # Validate that copy codec requires min_duration >= 1 for proper keyframe handling
    if args.copy and args.min_duration < 1:
        sys.stderr.write("Warning: --copy requires --min_duration to be at least 1 second for proper keyframe alignment\n")
        sys.stdout.flush()
    
    if not args.copy:
        sys.stdout.write("Using re-encoding method (may be slower but more reliable output)\n")
        sys.stdout.write("If you want faster but potentially glitchy output, use --copy\n")
        sys.stdout.flush()

    if args.verbose:
        sys.stdout.write(f"Silence duration........: {args.duration} s\n")
        sys.stdout.write(f"Minimum sounded duration: {args.min_duration} s\n")
        sys.stdout.write(f"Noise level.............: {args.noise} dB\n")
        sys.stdout.write(f"Copy codec..............: {args.copy}\n")
        sys.stdout.write(f"Pause...................: {args.pause}\n")
        sys.stdout.write(f"Threads.................: {args.threads}\n")
        sys.stdout.write(f"GPU.....................: {args.gpu}\n")
        sys.stderr.flush()

    return args
