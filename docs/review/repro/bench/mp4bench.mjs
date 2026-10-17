const MAX_BYTES = 50 * 1024 * 1024;
class Mp4Boxes {
  header = []; skip = 0; toEnd = false; boxes = 0; moov = false; bad = false;
  push(chunk) {
    let i = 0;
    while (!this.bad && i < chunk.length) {
      if (this.toEnd) return true;
      if (this.skip > 0) { const step = Math.min(this.skip, chunk.length - i); this.skip -= step; i += step; continue; }
      this.header.push(chunk[i++]);
      this.readHeader();
    }
    return !this.bad;
  }
  end() { return !this.bad && this.header.length === 0 && (this.skip === 0 || this.toEnd) && this.moov; }
  readHeader() {
    const h = this.header;
    if (h.length < 8) return;
    const size32 = ((h[0] << 24) >>> 0) + (h[1] << 16) + (h[2] << 8) + h[3];
    if (size32 === 1 && h.length < 16) return;
    const type = String.fromCharCode(h[4], h[5], h[6], h[7]);
    const size = size32 === 1 ? (h[8] * 2 ** 24 + (h[9] << 16) + (h[10] << 8) + h[11]) * 2 ** 32 + ((h[12] << 24) >>> 0) + (h[13] << 16) + (h[14] << 8) + h[15] : size32;
    const first = this.boxes === 0;
    this.boxes++;
    if (!/^[\x20-\x7e]{4}$/.test(type) || (first && (type !== "ftyp" || size < 16 || size > 1024))) { this.bad = true; return; }
    if (type === "moov") this.moov = true;
    if (size === 0) this.toEnd = true;
    else if (size < h.length || size > MAX_BYTES) this.bad = true;
    else this.skip = size - h.length;
    this.header = [];
  }
}
const total = 50 * 1024 * 1024 - 1;
const buf = new Uint8Array(total);
// ftyp 16 bytes
buf.set([0,0,0,16, 0x66,0x74,0x79,0x70, 0x69,0x73,0x6f,0x6d, 0,0,0,0], 0);
let off = 16;
let n = 0;
while (off + 8 <= total) {
  buf.set([0,0,0,8], off); buf.set(n === 0 ? [0x6d,0x6f,0x6f,0x76] : [0x66,0x72,0x65,0x65], off+4); off += 8; n++;
}
const used = buf.subarray(0, off);
const b = new Mp4Boxes();
const t0 = process.cpuUsage(); const w0 = performance.now();
const CH = 65536;
let ok = true;
for (let i = 0; i < used.length; i += CH) { if (!b.push(used.subarray(i, i + CH))) { ok = false; break; } }
const w1 = performance.now(); const t1 = process.cpuUsage(t0);
console.log({ boxes: b.boxes, ok, end: b.end(), wallMs: (w1-w0).toFixed(0), cpuMs: ((t1.user+t1.system)/1000).toFixed(0), perChunkMs: ((w1-w0)/(used.length/CH)).toFixed(2) });
// normal file: ftyp, moov 1MB, mdat rest
const nb = new Uint8Array(total);
nb.set([0,0,0,16, 0x66,0x74,0x79,0x70, 0x69,0x73,0x6f,0x6d, 0,0,0,0], 0);
const moovSize = 1<<20; nb.set([(moovSize>>>24)&255,(moovSize>>>16)&255,(moovSize>>>8)&255,moovSize&255, 0x6d,0x6f,0x6f,0x76], 16);
const mdatSize = total - 16 - moovSize; nb.set([(mdatSize>>>24)&255,(mdatSize>>>16)&255,(mdatSize>>>8)&255,mdatSize&255, 0x6d,0x64,0x61,0x74], 16+moovSize);
const b2 = new Mp4Boxes(); const w2 = performance.now();
for (let i = 0; i < nb.length; i += CH) b2.push(nb.subarray(i, i + CH));
console.log({ normalMs: (performance.now()-w2).toFixed(2), end: b2.end(), boxes: b2.boxes });
