const N = 1000;
const ts = Array.from({length: N}, (_, i) => Date.now() - (i + 30) * 3600_000 * 5);
function formatAgo(t){ return new Date(t).toLocaleDateString("en-US",{day:"numeric",month:"long",year:"numeric"}); }
const fmt = new Intl.DateTimeFormat("en-US",{day:"numeric",month:"long",year:"numeric"});
for (let r=0;r<3;r++){
 let t0=performance.now(); for (const t of ts) formatAgo(t); let t1=performance.now();
 for (const t of ts) fmt.format(t); let t2=performance.now();
 console.log(`toLocaleDateString x${N}: ${(t1-t0).toFixed(2)}ms  cached Intl: ${(t2-t1).toFixed(2)}ms`);
}
// haystack filter cost
const clips = Array.from({length:N},(_,i)=>({path:`C:\\Users\\u\\Videos\\ClippiBoy\\Game ${i%20}\\clip_2026-08-1${i%9}_08-30-00.mp4`,title:i%3?null:"Ace on Mirage "+i,description:i%5?null:"Last round 1v3 after the retake",game:"Game "+(i%20),tags:["ace","clutch"]}));
for (let r=0;r<3;r++){
 const q="mir"; let t0=performance.now();
 const v=clips.filter(c=>{const h=[c.path.split(/[\\/]/).pop(),c.title??"",c.description??"",c.game??"",...c.tags].join(" ").toLowerCase(); return h.includes(q.toLowerCase());});
 console.log(`filter x${N}: ${(performance.now()-t0).toFixed(2)}ms (${v.length})`);
}
