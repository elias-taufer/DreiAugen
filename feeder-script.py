import time
import datetime as dt
import subprocess
import signal
import sys
import os
import logging

import RPi.GPIO as GPIO

# ---------------------------
# Configuration
# ---------------------------

FEED_GPIO = 17
FEED_PULSE_MS = 1050
VIDEO_SECONDS_BEFORE = 4
VIDEO_DURATION_SECONDS = 300

FEED_TIMES = [
    (9, 0),
    (16, 0),
]

VIDEO_DIR = os.path.expanduser("~/feed_videos")
LOG_FILE = os.path.expanduser("~/feed_videos/feeder.log")

RPICAM_CMD_BASE = [
    "rpicam-vid",
    "--nopreview",
    "--width", "1920",
    "--height", "1080",
    "--framerate", "30",
    "--codec", "h264",
]

# ---------------------------
# Create folders
# ---------------------------

os.makedirs(VIDEO_DIR, exist_ok=True)

# ---------------------------
# Logging setup
# ---------------------------

logger = logging.getLogger("feeder")
logger.setLevel(logging.INFO)
logger.handlers.clear()

formatter = logging.Formatter("[%(asctime)s] %(message)s", "%Y-%m-%d %H:%M:%S")

console_handler = logging.StreamHandler(sys.stdout)
console_handler.setFormatter(formatter)

file_handler = logging.FileHandler(LOG_FILE)
file_handler.setFormatter(formatter)

logger.addHandler(console_handler)
logger.addHandler(file_handler)

# ---------------------------
# GPIO setup
# ---------------------------

GPIO.setmode(GPIO.BCM)
GPIO.setup(FEED_GPIO, GPIO.OUT, initial=GPIO.LOW)

# ---------------------------
# Helpers
# ---------------------------

def log(msg: str) -> None:
    logger.info(msg)

def make_output_filename(feed_time: dt.datetime) -> str:
    timestamp = feed_time.strftime("%Y-%m-%d_%H-%M-%S")
    return os.path.join(VIDEO_DIR, f"feeding_{timestamp}.h264")

def start_video(feed_time: dt.datetime) -> subprocess.Popen:
    output_file = make_output_filename(feed_time)
    cmd = RPICAM_CMD_BASE + [
        "--timeout", str(VIDEO_DURATION_SECONDS * 1000),
        "-o", output_file,
    ]
    log(f"Starting video: {' '.join(cmd)}")
    return subprocess.Popen(cmd)

def trigger_feed() -> None:
    log(f"Feeding: GPIO{FEED_GPIO} HIGH for {FEED_PULSE_MS} ms")
    GPIO.output(FEED_GPIO, GPIO.HIGH)
    time.sleep(FEED_PULSE_MS / 1000.0)
    GPIO.output(FEED_GPIO, GPIO.LOW)
    log("Feeding pulse finished")

def next_feed_datetime(now: dt.datetime) -> dt.datetime:
    candidates = []
    for hour, minute in FEED_TIMES:
        candidate = now.replace(hour=hour, minute=minute, second=0, microsecond=0)
        if candidate <= now:
            candidate += dt.timedelta(days=1)
        candidates.append(candidate)
    return min(candidates)

def cleanup(*_args) -> None:
    log("Cleaning up GPIO and exiting")
    GPIO.output(FEED_GPIO, GPIO.LOW)
    GPIO.cleanup()
    sys.exit(0)

# ---------------------------
# Main loop
# ---------------------------

def main() -> None:
    signal.signal(signal.SIGINT, cleanup)
    signal.signal(signal.SIGTERM, cleanup)

    log("Feeder scheduler started")

    while True:
        now = dt.datetime.now()
        feed_time = next_feed_datetime(now)
        video_start_time = feed_time - dt.timedelta(seconds=VIDEO_SECONDS_BEFORE)

        log(f"Next feeding at {feed_time.strftime('%Y-%m-%d %H:%M:%S')}")
        log(f"Video will start at {video_start_time.strftime('%Y-%m-%d %H:%M:%S')}")

        while dt.datetime.now() < video_start_time:
            time.sleep(0.5)

        video_proc = start_video(feed_time)

        while dt.datetime.now() < feed_time:
            time.sleep(0.05)

        trigger_feed()

        time.sleep(1)
        if video_proc.poll() is not None:
            log(f"Warning: video process already exited with code {video_proc.returncode}")

if __name__ == "__main__":
    main()
