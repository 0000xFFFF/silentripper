#!/usr/bin/env python3
import sys
import os
import subprocess
import time
import argparse

parser = argparse.ArgumentParser(description='remove silent parts from video using ffmpeg')
parser.add_argument('-d', '--duration', metavar='sec', type=float, default=1, help="silence duration in seconds (default: 1) (min: 1)")
parser.add_argument('-m', '--min_duration', metavar='sec', type=float, default=1, help="set min duration for each clip (default: 1)")
parser.add_argument('-n', '--noise', metavar='dB', type=int, default=-30, help="noise in dB (default: -30)")
parser.add_argument('-c', '--copy', action='store_true', help="use -c option for ffmpeg (fast copy but possible glitchy output)")
parser.add_argument('-p', '--pause', action='store_true', help="prompt before every action")
parser.add_argument('filename', type=argparse.FileType('r'))
args = parser.parse_args()

file_path = sys.argv[1]
file_split = os.path.splitext(file_path)
file_name = file_split[0]
file_ext  = file_split[1]

altered = 0
totaldur_muted   = 0
totaldur_sounded = 0
failed2conv = 0

addcopystring = ""
if args.copy:
    addcopystring = "-c copy"

if float(args.duration) < 1: args.duration = "1"
print(f"db: {args.noise}, duration: {args.duration}, copy: {args.copy}")

ffmpeg_sd = subprocess.Popen(
[
    'ffmpeg',
    '-hide_banner',
    '-vn',
    '-i',
    file_path,
    '-af',
    'silencedetect=n=' + str(args.noise) + 'dB:d=' + str(args.duration),
    '-f',
    'null',
    '-'
], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)

g_start = ""
g_end = ""
g_dur = ""
mutedtimestamps = []
for line_ in ffmpeg_sd.stdout.read().decode().split("\n"):
    line = line_.replace("\r", "")
    if not line: continue
    if not "silencedetect" in line: continue
    if "silence_start: " in line:
        g_start = line.split("silence_start: ")[1]
        continue
    if "silence_end: " in line:
        endNdur = line.split("silence_end: ")[1]
        g_end = endNdur.split(" | ")[0]
        g_dur = endNdur.split("silence_duration: ")[1]
        mutedtimestamps.append([g_start,g_end,g_dur])
        continue

#'''
print("===[ MUTED TIMESTAMPS (START, END, DURATION) ]===")
counter = 0
for i in mutedtimestamps:
    counter += 1
    totaldur_muted += float(i[2])
    print(counter,i[0],i[1],i[2])
#'''

mutedtimestamps_len = len(mutedtimestamps)
subvideos = []

def subvideos_append(s,e):
    global altered
    o = "p" + str(len(subvideos)+1) + ".mts"
    d = 0
    if s and e:
        d = round(float(e) - float(s), 5)
        if args.min_duration and d < args.min_duration:
            altered += 1
            d2add = args.min_duration - d
            e = str(round(float(e) + d2add, 5))
            d = round(float(e) - float(s), 5)

    subvideos.append([s,e,str(d),o])


if mutedtimestamps_len >= 1: # add beggining
    start = "0"
    end = mutedtimestamps[0][0]
    if start != end: subvideos_append(start,end)
for i in range(mutedtimestamps_len):
    s = mutedtimestamps[i][1]
    ii = i + 1
    e = ""
    if ii < mutedtimestamps_len:
        e = mutedtimestamps[ii][0]
    if s != e:
        subvideos_append(s,e)

print("===[ SOUNDED TIMESTAMPS (FILETOWRITE, START, END, DURATION) ]===")
counter = 0
subvideos_len = len(subvideos)
for i in subvideos:
    counter += 1
    totaldur_sounded += float(i[2])
    print(counter,i[3],i[0],i[1],i[2])

def convTime(sec):
    ty_res = time.gmtime(sec)
    res = time.strftime("%H:%M:%S",ty_res)
    return res

print(f"muted: {mutedtimestamps_len}")
print(f"sound: {subvideos_len}")
print(f"altered: {altered}")
print(f"total silence duration: {convTime(totaldur_muted)} ({totaldur_muted:.2f}s)")
print(f"total sounded duration: {convTime(totaldur_sounded)} ({totaldur_sounded:.2f}s)")
print()

if args.pause:
    try:
        input("Hit enter to: start making cuts")
    except KeyboardInterrupt:
        print("\n")
        exit()

print()

file_out = open("list.txt", "w")

print("===[ MAKING CUTS ]===")
subvideos_counter = 0
subvideos_len = len(subvideos)
for i in subvideos:
    start = i[0]
    end = i[1]
    dur = i[2]
    outfile = i[3]
    subvideos_counter += 1

    startcmd = ""
    endcmd = ""
    if start: startcmd = " -ss " + start
    if end: endcmd = " -to " + end

    s = "ffmpeg -hide_banner -v quiet -i \"" + file_path + "\"" + startcmd + endcmd + " " + addcopystring + " " + outfile
    print(f"{subvideos_counter}/{subvideos_len} ({subvideos_counter/subvideos_len*100:.2f}%) {s} --> {dur} --> ...", end='')
    x = os.system(s)
    print("\b\b\b", end="")
    if x == 0:
        print("SUCCESS")
        file_out.write("file " + outfile + "\n")
    else:
        failed2conv += 1
        print("FAILED")


file_out.close()

print(f"files failed to convert: {failed2conv}")

# combine videos
print("===[ CONCAT ]===")
run_concat = "ffmpeg -hide_banner -loglevel error -f concat -i list.txt " + addcopystring + " \"" + file_name + "_cut" + file_ext + "\""
print(run_concat)
concat_rc = os.system(run_concat)

if args.pause:
    print("Hit enter to: cleanup (del p* list.txt)")
    input()

# cleanup
for i in subvideos:
    os.remove(i[3])
os.remove("list.txt")

