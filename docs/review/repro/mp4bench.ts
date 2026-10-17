import { Mp4Boxes, MAX_BYTES } from "./shares.ts";
const box = (type: string, body: number[] = []) => {
  const size = 8 + body.length;
  return [size >>> 24, (size >> 16) & 255, (size >> 8) & 255, size & 255, ...Array.from(type, (c) => c.charCodeAt(0)), ...body];
};
const ftyp = box("ftyp", Array.from("isom\0\0\0\0isommp41", (c) => c.charCodeAt(0)));
// 1) polyglot: ftyp + moov + mdat(size 0) + HTML
{
  const html = Array.from("<html><script>alert(1)</script></html>", (c) => c.charCodeAt(0));
  const file = [...ftyp, ...box("moov"), 0,0,0,0, ...Array.from("mdat",(c)=>c.charCodeAt(0)), ...html];
  const b = new Mp4Boxes(); console.log("polyglot push", b.push(new Uint8Array(file)), "end", b.end());
}
// 2) CPU: 50 MB of 8-byte boxes
{
  const total = MAX_BYTES;
  const buf = new Uint8Array(total);
  let o = 0;
  buf.set(ftyp, o); o += ftyp.length;
  const moov = box("moov"); buf.set(moov, o); o += moov.length;
  const free = box("free");
  while (o + 8 <= total) { buf.set(free, o); o += 8; }
  const real = o;
  const b = new Mp4Boxes();
  const t0 = performance.now();
  const CH = 64 * 1024;
  let ok = true;
  for (let i = 0; i < real; i += CH) { if (!b.push(buf.subarray(i, Math.min(real, i + CH)))) { ok = false; break; } }
  const t1 = performance.now();
  console.log("tiny boxes: ok", ok, "end", b.end(), "ms", (t1 - t0).toFixed(0), "boxes", (real / 8) | 0);
}
// 3) CPU: a normal-ish file of one big mdat
{
  const total = MAX_BYTES;
  const buf = new Uint8Array(total);
  let o = 0; buf.set(ftyp, o); o += ftyp.length;
  const moov = box("moov"); buf.set(moov, o); o += moov.length;
  const sz = total - o; buf.set([sz>>>24,(sz>>16)&255,(sz>>8)&255,sz&255,0x6d,0x64,0x61,0x74], o);
  const b = new Mp4Boxes(); const t0 = performance.now();
  for (let i = 0; i < total; i += 65536) b.push(buf.subarray(i, Math.min(total, i + 65536)));
  console.log("one mdat: end", b.end(), "ms", (performance.now() - t0).toFixed(2));
}
