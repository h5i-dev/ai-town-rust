import { test } from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { homedir } from 'node:os';
import { resolve } from 'node:path';
import { readFileSync } from 'node:fs';
const upstream=resolve(process.env.AI_TOWN_UPSTREAM??`${homedir()}/Ref/ai-town`);
const bundle=await build({stdin:{contents:`export {HistoricalObject,unpackSampleRecord} from ${JSON.stringify(resolve(upstream,'convex/engine/historicalObject.ts'))};
export {decodeLocation,historicalValue,TownPlayback} from ${JSON.stringify(resolve('frontend/src/history.ts'))};
export {poseAt} from ${JSON.stringify(resolve('frontend/src/pose.ts'))};`,resolveDir:process.cwd()},bundle:true,platform:'node',format:'esm',write:false});
const {HistoricalObject,unpackSampleRecord,decodeLocation,historicalValue,TownPlayback,poseAt}=await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);
const fields=[{name:'x',precision:8},{name:'y',precision:8},{name:'dx',precision:8},{name:'dy',precision:8},{name:'speed',precision:16}];
test('browser decoder matches upstream on varied actual native packets',()=>{
  let seed=13;const random=()=>((seed=(Math.imul(seed,1664525)+1013904223)>>>0)/2**32);
  for(let round=0;round<100;round++) {
    const base={x:1,y:2,dx:1,dy:0,speed:0},h=new HistoricalObject(fields,base);
    for(let i=1;i<=60;i++) {
      const next={...h.data};
      for(const f of fields)if(random()<.45)next[f.name]=Math.floor(random()*2000)/1000*(f.name==='dx'||f.name==='dy'?-1:1);
      h.update(1000000+16*i,next);
    }
    const packet=h.pack(), native=unpackSampleRecord(fields,packet);
    const track=decodeLocation(Buffer.from(packet).toString('base64'),1000000,1000960);
    fields.forEach((f,i)=>{
      assert.equal(track[i]?.initial,native[f.name]?.initialValue);
      assert.deepEqual(track[i]?.times,native[f.name]?.samples.map(s=>s.time));
      assert.deepEqual(track[i]?.values,native[f.name]?.samples.map(s=>s.value));
    });
  }
});
test('rendered pose obeys actual ticks, turns and stops even if a future path exists',()=>{
  const h=new HistoricalObject(fields,{x:1,y:2,dx:1,dy:0,speed:0});
  h.update(16,{x:1.012,y:2,dx:-1,dy:0,speed:.75});h.update(32,{x:1.012,y:2,dx:-1,dy:0,speed:0});
  const world={nowMs:32,players:[{id:1,x:1012,y:2000,faceDx:0,faceDy:1000,speed:0,path:[{x:0,y:0,t:0},{x:999999,y:0,t:1000}]}],
    motionHistory:{startMs:0,endMs:32,locations:[{playerId:1,buffer:Buffer.from(h.pack()).toString('base64')}]}};
  const p=new TownPlayback();p.receive(world,0);const player=p.sample(0).world.players[0];
  assert.deepEqual(poseAt(player,0),{x:1,y:2,faceDx:2000,faceDy:1000,speed:0});
  assert.equal(poseAt(player,16).speed,.75);assert.equal(poseAt(player,16).faceDx,0);
  assert.equal(poseAt(player,32).speed,0);assert.equal(poseAt(player,999999).x,259/256);
});
test('native quantization can produce a 61-value run including the initial value',()=>{
  const h=new HistoricalObject(fields,{x:0,y:0,dx:1,dy:0,speed:0});
  for(let i=1;i<=60;i++)h.update(i*16,{...h.data,x:i%2/1000});
  const track=decodeLocation(Buffer.from(h.pack()).toString('base64'),0,960);
  assert.equal(track[0].times.length,60);
  assert(track[0].values.every(value=>value===0));
});
if(process.env.AI_TOWN_HISTORY_FRAME) test('Rust demo packets decode identically upstream and reach the authoritative end position',()=>{
  const world=JSON.parse(readFileSync(process.env.AI_TOWN_HISTORY_FRAME,'utf8')),history=world.motionHistory;
  assert(history&&history.locations.length>0);
  const byId=new Map(world.players.map(p=>[p.id,p]));let count=0;
  for(const l of history.locations) {
    const bytes=Uint8Array.from(Buffer.from(l.buffer,'base64'));
    const native=unpackSampleRecord(fields,bytes.buffer),track=decodeLocation(l.buffer,history.startMs,history.endMs);
    fields.forEach((f,i)=>{if(native[f.name])assert.deepEqual(track[i],{initial:native[f.name].initialValue,times:native[f.name].samples.map(s=>s.time),values:native[f.name].samples.map(s=>s.value)});});
    const player=byId.get(l.playerId);if(!player)continue;
    assert(Math.abs(historicalValue(track[0],history.endMs,player.x/1000)-player.x/1000)<1/256);
    assert(Math.abs(historicalValue(track[1],history.endMs,player.y/1000)-player.y/1000)<1/256);count++;
  }
  assert(count>0);
});
