"""Brightness-only write test for Ulanzi D200H (2207:0019). Sends command 0x000A and nothing else."""
import hid, sys, time

VID, PID = 0x2207, 0x0019

def packet(cmd: int, payload: bytes) -> bytes:
    body = b"\x7c\x7c" + cmd.to_bytes(2, "big") + len(payload).to_bytes(4, "little") + payload
    return b"\x00" + body.ljust(1024, b"\x00")  # leading 0x00 = report ID

infos = [d for d in hid.enumerate(VID, PID) if d["interface_number"] == 0]
dev = hid.device()
dev.open_path(infos[0]["path"])

def set_brightness(v: int):
    n = dev.write(packet(0x000A, str(max(0, min(100, v))).encode()))
    print(f"brightness {v}: wrote {n} bytes", flush=True)
    return n

set_brightness(20)
time.sleep(3)
set_brightness(86)  # restore saved value
dev.close()
