'use strict';
// Frida hook for UlanziDeck.exe: logs HID traffic to the deck (1024/1025-byte reports) via kernel32.
// Writes to D:\My Deck App\captures\ : traffic.log (every packet) and zip_N_cmdXXXX.zip (reassembled uploads).
const OUT = 'D:\\My Deck App\\captures\\';
const log = new File(OUT + 'traffic.log', 'w');
const t0 = Date.now();
let zipCount = 0;
let pending = null; // { cmd, total, parts: [], got }

function L(s) { log.write(((Date.now() - t0) / 1000).toFixed(3) + ' ' + s + '\n'); log.flush(); }
function hex(buf, n) {
  const u = new Uint8Array(buf, 0, Math.min(n, buf.byteLength));
  return Array.prototype.map.call(u, b => ('0' + b.toString(16)).slice(-2)).join(' ');
}
function ascii(u8) {
  let s = '';
  for (let i = 0; i < u8.length; i++) { const c = u8[i]; if (c === 0) break; s += (c >= 32 && c < 127) ? String.fromCharCode(c) : '.'; }
  return s;
}

function finishZip() {
  if (!pending) return;
  const all = new Uint8Array(pending.got);
  let o = 0;
  pending.parts.forEach(p => { all.set(p, o); o += p.length; });
  const name = OUT + 'zip_' + (++zipCount) + '_cmd' + ('0000' + pending.cmd.toString(16)).slice(-4) + '.zip';
  const f = new File(name, 'wb'); f.write(all.buffer.slice(0, pending.got)); f.flush(); f.close();
  L('ZIP saved ' + name + ' bytes=' + pending.got + '/' + pending.total);
  send('ZIP saved ' + name + ' bytes=' + pending.got);
  pending = null;
}

// pkt: ArrayBuffer of the 1024-byte protocol packet (report ID already stripped)
function onPacket(dir, pkt) {
  const u = new Uint8Array(pkt);
  const isCmd = u[0] === 0x7c && u[1] === 0x7c;
  if (dir === 'W' && pending && !isCmd) { // continuation chunk of a ZIP
    const take = Math.min(1024, pending.total - pending.got);
    pending.parts.push(u.slice(0, take)); pending.got += take;
    if (pending.got >= pending.total) finishZip();
    return;
  }
  if (!isCmd) { L(dir + ' non-cmd ' + hex(pkt, 24)); return; }
  const cmd = (u[2] << 8) | u[3];
  const len = u[4] | (u[5] << 8) | (u[6] << 16) | (u[7] << 24);
  const payload = u.slice(8, 8 + Math.min(len, 1016));
  const c = '0x' + ('0000' + cmd.toString(16)).slice(-4);
  if (dir === 'R' && (cmd === 0x0101 || cmd === 0x0102)) { L('R ' + c + ' button ' + hex(payload.buffer, 4)); return; }
  L(dir + ' ' + c + ' len=' + len + ' ascii="' + ascii(payload).slice(0, 200) + '" hex=' + hex(payload.buffer, 24));
  send(dir + ' ' + c + ' len=' + len + ' ' + ascii(payload).slice(0, 120));
  if (dir === 'W' && (cmd === 0x0001 || cmd === 0x000d)) {
    pending = { cmd: cmd, total: len, parts: [payload], got: payload.length };
    if (pending.got >= pending.total) finishZip();
  }
}

const k32 = Process.getModuleByName('kernel32.dll');

Interceptor.attach(k32.getExportByName('WriteFile'), {
  onEnter(args) {
    const n = args[2].toInt32();
    if (n !== 1025 && n !== 1024) return;
    const raw = args[1].readByteArray(n);
    onPacket('W', n === 1025 ? raw.slice(1) : raw);
  }
});

// hidapi uses overlapped reads: remember the buffer, read it when the overlapped result completes
const reads = {};
Interceptor.attach(k32.getExportByName('ReadFile'), {
  onEnter(args) {
    this.n = args[2].toInt32(); this.buf = args[1]; this.ov = args[4];
  },
  onLeave(ret) {
    if (this.n !== 1025 && this.n !== 1024) return;
    if (this.ov && !this.ov.isNull()) { reads[this.ov.toString()] = { buf: this.buf, n: this.n }; }
  }
});
Interceptor.attach(k32.getExportByName('GetOverlappedResult'), {
  onEnter(args) { this.ov = args[1].toString(); this.cnt = args[2]; },
  onLeave(ret) {
    const r = reads[this.ov];
    if (!r || ret.toInt32() === 0) return;
    const got = this.cnt.isNull() ? r.n : this.cnt.readU32();
    if (got < 12) return;
    const raw = r.buf.readByteArray(got);
    onPacket('R', got === 1025 ? raw.slice(1) : raw);
  }
});
send('hooks installed; logging to ' + OUT);
