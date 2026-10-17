import { Mp4Boxes } from "./shares.ts";
const box = (type: string, body: number[] = [], sz?: number) => {
  const size = sz ?? 8 + body.length;
  return [size >>> 24, (size >> 16) & 255, (size >> 8) & 255, size & 255, ...Array.from(type, (c) => c.charCodeAt(0)), ...body];
};
const ftyp = box("ftyp", Array.from("isom\0\0\0\0isommp41", (c) => c.charCodeAt(0)));
const html = Array.from("<html><script>alert(1)</script></html>", c => c.charCodeAt(0));
const walk = (bytes: number[]) => { const b = new Mp4Boxes(); if (!b.push(new Uint8Array(bytes))) return false; return b.end(); };
console.log("size0 mdat + html:", walk([...ftyp, ...box("moov", [1]), ...box("mdat", [], 0), ...html]));
console.log("free box with html:", walk([...ftyp, ...box("moov", [1]), ...box("free", html)]));
console.log("moov size0 + html:", walk([...ftyp, ...box("moov", [], 0), ...html]));
