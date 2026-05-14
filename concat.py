import sys
import subprocess
from globals import Variables

def concat_prepare_list(vars: Variables):
    with open("clip_list.txt", "w") as clip_list_file:
        for clip in vars.sounded_sections:
            if clip["success"]:
                clip_list_file.write(f"file '{clip['file']}'\n")

def concat(vars: Variables):

    assert vars.args is not None

    concat_cmd = [
        'ffmpeg', '-hide_banner', '-loglevel', 'error',
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

    result = subprocess.run(concat_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode != 0:
        sys.stdout.write(f"Concatenation failed with error: {result.stderr.decode('utf-8')}\n")
        sys.stdout.flush()
    else:
        sys.stdout.write("Concatenation succeeded!\n")
        sys.stdout.flush()
