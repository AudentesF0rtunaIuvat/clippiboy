const BASE = "http://127.0.0.1:8799";
const T = { a: "tok-a", b: "tok-b", c: "tok-c" };
const ID = { a: "00000000-0000-4000-8000-00000000000a", b: "00000000-0000-4000-8000-00000000000b", c: "00000000-0000-4000-8000-00000000000c" };
const api = async (tok, method, path, body, extra = {}) => {
  const res = await fetch(BASE + path, { method, headers: { Authorization: `Bearer ${tok}`, ...extra.headers }, body });
  const text = await res.text();
  return { status: res.status, text: text.slice(0, 200), headers: res.headers };
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
function sock(tok) {
  const ws = new WebSocket(BASE.replace("http", "ws") + "/ws", { headers: { Authorization: `Bearer ${tok}` } });
  const inbox = [];
  ws.onmessage = (e) => inbox.push(e.data === "pong" ? "pong" : JSON.parse(e.data));
  return new Promise((res, rej) => { ws.onopen = () => res({ ws, inbox }); ws.onerror = (e) => rej(e); });
}
// 1. banned user C still authenticated
console.log("banned GET /me", (await api(T.c, "GET", "/me")).status);
// 2. banned C websocket + relay to friend A
const sa = await sock(T.a); sa.ws.send(JSON.stringify({ t: "hello", invisible: false }));
const sc = await sock(T.c); sc.ws.send(JSON.stringify({ t: "hello", invisible: false, game: "Banned Game" }));
await sleep(500);
sc.ws.send(JSON.stringify({ t: "relay", to: ID.a, body: { id: "from-banned", kind: "share-offer" } }));
await sleep(500);
console.log("A got relay from banned C:", sa.inbox.some((m) => m.t === "relay" && m.from === ID.c), "A saw C presence:", JSON.stringify(sa.inbox.find((m) => m.t === "presence" && m.id === ID.c)?.presence ?? sa.inbox.find(m=>m.t==="snapshot")?.friends?.[ID.c]));
// 3. invisible probe: B invisible, A relays to B
const sb = await sock(T.b); sb.ws.send(JSON.stringify({ t: "hello", invisible: true }));
await sleep(500);
sa.inbox.length = 0;
sa.ws.send(JSON.stringify({ t: "relay", to: ID.b, body: { id: "probe" } }));
await sleep(500);
console.log("A snapshot/presence of invisible B shows:", JSON.stringify(sa.inbox.filter(m=>m.t==="presence"&&m.id===ID.b).map(m=>m.presence)), "relayFailed:", JSON.stringify(sa.inbox.filter((m) => m.t === "relayFailed")), "B received probe:", sb.inbox.some((m) => m.t === "relay" && m.body?.id === "probe"));
// 4. relay budget per connection: 3 extra sockets of A, 30 each
const extra = await Promise.all([sock(T.a), sock(T.a), sock(T.a)]);
for (const s of extra) s.ws.send(JSON.stringify({ t: "hello", invisible: false }));
await sleep(300);
sb.inbox.length = 0;
for (const s of [sa, ...extra]) for (let i = 0; i < 30; i++) s.ws.send(JSON.stringify({ t: "relay", to: ID.b, body: { id: "n" + i } }));
await sleep(2000);
console.log("relays delivered to B within ~2s from 4 sockets:", sb.inbox.filter((m) => m.t === "relay").length);
// 5. presence flip flood (no limit)
const before = sb.inbox.length;
for (let i = 0; i < 200; i++) sa.ws.send(JSON.stringify({ t: "presence", invisible: false, game: i % 2 ? "x" : "y" }));
await sleep(2000);
console.log("presence msgs B received from 200 flips:", sb.inbox.slice(before).filter((m) => m.t === "presence").length);
// 6. null JSON body
console.log("exchange null body:", (await fetch(BASE + "/auth/exchange", { method: "POST", body: "null" })).status);
console.log("PATCH /me null body:", (await api(T.a, "PATCH", "/me", "null")).status);
// 7. banned C deletes own account -> ban gone
console.log("banned DELETE /me", (await api(T.c, "DELETE", "/me")).status);
for (const s of [sa, sb, sc, ...extra]) s.ws.close();
process.exit(0);
