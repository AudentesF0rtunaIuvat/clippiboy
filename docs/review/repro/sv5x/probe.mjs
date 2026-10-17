try { await import("server/src/page.ts"); console.log("page.ts imports OK"); } catch (e) { console.log("page.ts import fails:", e.code, e.message.split("\n")[0]); }
const { Mp4Boxes } = await import("server/src/shares.ts");
const be32 = (n) => [n >>> 24 & 255, n >> 16 & 255, n >> 8 & 255, n & 255];
const box = (t, body = []) => [...be32(8 + body.length), ...[...t].map(c => c.charCodeAt(0)), ...body];
const ftyp = box("ftyp", [..."isom\0\0\0\0isommp41"].map(c => c.charCodeAt(0)));
const walk = (bytes) => { const b = new Mp4Boxes(); if (!b.push(new Uint8Array(bytes))) return false; return b.end(); };
console.log("size0 moov to end:", walk([...ftyp, 0,0,0,0, ...[..."moov"].map(c=>c.charCodeAt(0)), 1,2,3]));
console.log("largesize mdat + moov:", walk([...ftyp, ...box("moov"), 0,0,0,1, ...[..."mdat"].map(c=>c.charCodeAt(0)), 0,0,0,0,0,0,0,18, 9,9]));
console.log("size 4 box:", walk([...ftyp, ...box("moov"), 0,0,0,4, 0x6d,0x64,0x61,0x74]));
console.log("ftyp 1100:", walk([...box("ftyp", new Array(1092).fill(0x61)), ...box("moov")]));
