const o={day:"numeric",month:"long",year:"numeric"};
const base=Date.now()-86400000*30;
for (let r=0;r<5;r++){
let t=performance.now(); let s=0;
for(let i=0;i<1000;i++) s+=new Date(base-i*3600000).toLocaleDateString("en-US",o).length;
const a=performance.now()-t;
const F=new Intl.DateTimeFormat("en-US",o);
t=performance.now();
for(let i=0;i<1000;i++) s+=F.format(new Date(base-i*3600000)).length;
const b=performance.now()-t;
console.log(a.toFixed(1),"ms vs",b.toFixed(1),"ms",s);
}
