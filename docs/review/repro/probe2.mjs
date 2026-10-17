const BASE = "http://127.0.0.1:8799";
const box = (type, body = []) => { const size = 8 + body.length; return [size >>> 24, (size >> 16) & 255, (size >> 8) & 255, size & 255, ...Array.from(type, (c) => c.charCodeAt(0)), ...body]; };
const ftyp = box("ftyp", Array.from("isom\0\0\0\0isommp41", (c) => c.charCodeAt(0)));
const html = Array.from("<html><body><script>alert(document.domain)</script></body></html>", (c) => c.charCodeAt(0));
const polyglot = new Uint8Array([...ftyp, ...box("moov"), 0, 0, 0, 0, ...Array.from("mdat", (c) => c.charCodeAt(0)), ...html]);
const up = (tok, data, q = "title=%3Cscript%3Ealert(1)%3C%2Fscript%3E%22%27&game=%3Cimg%20src%3Dx%20onerror%3Dalert(1)%3E&tag=%3C%2Ftitle%3E&name=1") =>
  fetch(`${BASE}/shares?${q}`, { method: "POST", headers: { Authorization: "Bearer tok-b", "Content-Length": String(data.length) }, body: data }).then(async (r) => ({ status: r.status, body: await r.json().catch(() => null) }));
// parallel race: 6 at once; B has 0 used
const results = await Promise.all(Array.from({ length: 6 }, () => up("tok-b", polyglot)));
console.log("parallel statuses", results.map((r) => r.status).join(","));
const ok = results.find((r) => r.status === 200);
const id = ok.body.id;
console.log("polyglot accepted as share", id);
let r = await fetch(`${BASE}/c/${id}.mp4`);
console.log("media", r.status, r.headers.get("content-type"), r.headers.get("x-content-type-options"), r.headers.get("cache-control"), r.headers.get("content-disposition"), r.headers.get("x-robots-tag"));
r = await fetch(`${BASE}/c/${id}.mp4`, { headers: { Range: "bytes=999999-" } });
console.log("unsatisfiable range ->", r.status, (await r.text()).slice(0, 100));
r = await fetch(`${BASE}/c/${id}.mp4`, { headers: { Range: "bytes=0-1,5-6" } });
console.log("multi range ->", r.status, r.headers.get("content-range"));
r = await fetch(`${BASE}/c/${id}.mp4`, { headers: { Range: "bytes=garbage" } });
console.log("garbage range ->", r.status);
r = await fetch(`${BASE}/c/${id}.mp4`, { headers: { "If-None-Match": (await fetch(`${BASE}/c/${id}.mp4`)).headers.get("etag") } });
console.log("If-None-Match ->", r.status);
const page = await (await fetch(`${BASE}/c/${id}`)).text();
console.log("page escaped:", !page.includes("<script>alert(1)") && !page.includes("<img src=x"), "csp header:", (await fetch(`${BASE}/c/${id}`)).headers.get("content-security-policy"));
console.log(page.match(/<title>.*<\/title>/)[0]);
console.log(page.match(/<h1>.*?<\/h1>/)[0], page.match(/<div class="pills">.*?<\/div>/)?.[0]);
// poster: JPEG magic + HTML, check headers
const jpg = new Uint8Array([0xff, 0xd8, 0xff, ...html]);
r = await fetch(`${BASE}/shares/${id}/poster`, { method: "PUT", headers: { Authorization: "Bearer tok-b", "Content-Length": String(jpg.length) }, body: jpg });
console.log("poster put", r.status);
r = await fetch(`${BASE}/c/${id}.jpg`);
console.log("poster", r.status, r.headers.get("content-type"), r.headers.get("x-content-type-options"));
// A tries to delete B's share
r = await fetch(`${BASE}/shares/${id}`, { method: "DELETE", headers: { Authorization: "Bearer tok-a" } });
console.log("A deletes B share ->", r.status);
// ping with chunked body > 1KB, no content-length
const big = new ReadableStream({ start(c) { c.enqueue(new TextEncoder().encode("x".repeat(5_000_000))); c.close(); } });
r = await fetch(`${BASE}/t/ping`, { method: "POST", body: big, duplex: "half" });
console.log("chunked 5MB ping ->", r.status);
// unauth endpoints writing D1
r = await fetch(`${BASE}/auth/start?challenge=${"a".repeat(43)}`, { redirect: "manual" });
console.log("auth/start", r.status, r.headers.get("set-cookie"));
console.log("robots.txt", (await fetch(`${BASE}/robots.txt`)).status);
process.exit(0);
