// Node 22.18+ strips the type-only syntax of the browser-independent decoder.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { decodeLocation, historicalValue, TownPlayback } from '../src/history.ts';
const packet = Buffer.from([187,242,5,8,0,80,66,15,0,0,0,0,0,2,0,32,2,4,0,128,4,6,6]).toString('base64');
const start = 1_000_000;
function world(from=start, end=start+32) {
  return {nowMs:end,players:[{id:1,x:1024,y:0,faceDx:2000,faceDy:1000,speed:0,path:[{x:999999,y:0}]}],
    motionHistory:{startMs:from,endMs:end,locations:[{playerId:1,buffer:packet}]}};
}
test('upstream packet decodes quantized tick samples, never interpolates a planned path',()=>{
  const track=decodeLocation(packet,start,start+32);
  assert.equal(historicalValue(track[0],start,0),1);
  assert.equal(historicalValue(track[0],start+15,0),1);
  assert.equal(historicalValue(track[0],start+16,0),259/256);
  assert.equal(historicalValue(track[0],start+31,0),259/256);
  assert.equal(historicalValue(track[0],start+32,0),262/256);
});
test('facing and speed retain starts, turns and stops',()=>{
  const bytes=[187,242,5,8,2,80,66,15,0,0,0,0,0,0,0,4,0,128,4,255,7,4,80,66,15,0,0,0,0,0,2,0,32,2,7,0,0,128,128,6,255,255,5];
  const track=decodeLocation(Buffer.from(bytes).toString('base64'),start,start+32);
  assert.equal(historicalValue(track[2],start,0),1);
  assert.equal(historicalValue(track[2],start+16,0),-1);
  assert.equal(historicalValue(track[4],start,9),0);
  assert.equal(historicalValue(track[4],start+16,9),.75);
  assert.equal(historicalValue(track[4],start+32,9),0);
});
test('accepts 61 identical quantized values: initial value plus 60 changed raw samples',()=>{
  const bytes=[187,242,5,8,16,16,0,0,0,0,0,0,0,2,0,32,118,2,0,0,122];
  const track=decodeLocation(Buffer.from(bytes).toString('base64'),0,960);
  assert.equal(track[0].initial,0);
  assert.equal(track[0].times.length,60);
  assert.equal(track[0].times[59],960);
  assert(track[0].values.every(value=>value===0));
});
test('rejects truncated packets, wrong configuration, invalid runs and out-of-interval samples',()=>{
  const bytes=Buffer.from(packet,'base64');
  for(let n=0;n<bytes.length;n++) {
    // Four bytes alone describe an empty record, which is valid.
    if(n!==4)assert.throws(()=>decodeLocation(bytes.subarray(0,n).toString('base64'),start,start+32));
  }
  const wrong=Buffer.from(bytes);wrong[0]=0;assert.throws(()=>decodeLocation(wrong.toString('base64'),start,start+32));
  assert.throws(()=>decodeLocation(packet,start+17,start+32));
  const run=Buffer.from(bytes);run[16]=124;assert.throws(()=>decodeLocation(run.toString('base64'),start,start+32));
});
test('playback holds at last computed tick and duplicates do not restart time',()=>{
  const p=new TownPlayback();p.receive(world(),100);
  let sample=p.sample(120);assert(sample.now>start&&sample.now<start+32);
  const cursor=sample.now;p.receive(world(),120);
  assert.equal(p.sample(120).now,cursor);
  assert.equal(p.sample(10000).now,start+32);
  assert(sample.world.players[0].motion);
});
test('stationary residents receive empty tracks, suppressing future-path fallback',()=>{
  const p=new TownPlayback(),w=world();w.motionHistory.locations=[];p.receive(w,0);
  assert.deepEqual(p.sample(0).world.players[0].motion,[]);
});
test('gaps skip unrecorded time; reconnect to an older world resets playback',()=>{
  const p=new TownPlayback();p.receive(world(),0);p.sample(1000);
  const w=world(start+100,start+132);w.motionHistory.locations=[];p.receive(w,1000);
  assert(p.sample(1001).now>=start+100);
  p.receive(world(),1100);assert.equal(p.sample(1100).now,start);
});
test('retention and playback lag stay bounded after a suspended tab',()=>{
  const p=new TownPlayback();
  for(let i=0;i<100;i++) {const w=world(start+i*960,start+(i+1)*960);w.motionHistory.locations=[];p.receive(w,i);}
  const s=p.sample(100);assert(s.now>=start+96000-1500);assert(s.now<=start+96000);
  assert(p.frames.length<=8);
});
