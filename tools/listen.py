"""Read-only listener for Ulanzi D200H (2207:0019). Never writes to the device."""
import hid, sys, time

VID, PID = 0x2207, 0x0019
DURATION = float(sys.argv[1]) if len(sys.argv) > 1 else 40

infos = [d for d in hid.enumerate(VID, PID) if d["interface_number"] == 0]
if not infos:
    sys.exit("interface 0 not found")
dev = hid.device()
dev.open_path(infos[0]["path"])
dev.set_nonblocking(True)
print("opened", infos[0]["product_string"], "- listening", DURATION, "s (read-only)", flush=True)

t0 = time.time()
n = 0
while time.time() - t0 < DURATION:
    data = dev.read(1024)
    if data:
        n += 1
        b = bytes(data)
        ok = b[:2] == b"\x7c\x7c"
        cmd = int.from_bytes(b[2:4], "big") if len(b) >= 4 else None
        length = int.from_bytes(b[4:8], "little") if len(b) >= 8 else None
        print(f"[{time.time()-t0:6.2f}s] len={len(b)} hdr7c7c={ok} cmd={cmd:#06x} plen={length} payload={b[8:24].hex(' ')}", flush=True)
    else:
        time.sleep(0.01)
dev.close()
print("done, reports:", n, flush=True)
