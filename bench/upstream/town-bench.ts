import { Town } from './town';
import { appendFileSync, writeFileSync } from 'node:fs';
import assert from 'node:assert/strict';
const [fixture,output,s='3',t='60',stepsArg='1']=process.argv.slice(2);
const samples=Number(s), ticks=Number(t), steps=Number(stepsArg);
assert(Number.isInteger(samples)&&samples>0&&Number.isInteger(ticks)&&ticks>0&&ticks<=60&&Number.isInteger(steps)&&steps>0);
console.log=console.debug=console.warn=()=>{};
writeFileSync(output,'engine,players,sample,ticks,simulated_ms,loop_ms,history_pack_ms,snapshot_ms,moved,decisions,destinations,activities,invitations,messages,memories,pathfinds,history_bytes\n');
for(let sample=-1;sample<samples;sample++) {
  const town=new Town(fixture), before=[...town.game.world.players.values()].map(p=>({...p.position}));
  let loopMs=0, packMs=0, now=town.fixture.now;
  for(let step=0;step<steps;step++) {const result=town.step(now,ticks);now+=ticks*16;loopMs+=result.tickMs;packMs+=result.packMs;}
  let moved=0, i=0;
  const grid=new Map<string,any[]>();
  for(const p of town.game.world.players.values()) {
    if(p.position.x!==before[i].x||p.position.y!==before[i].y)moved++;
    const {x,y}=p.position, m=town.fixture.map;
    assert(x>=0&&y>=0&&x<m.width&&y<m.height);
    assert(m.objectTiles.every((l:any)=>l[Math.floor(x)][Math.floor(y)]===-1));
    const cx=Math.floor(x),cy=Math.floor(y);
    for(let dx=-1;dx<=1;dx++)for(let dy=-1;dy<=1;dy++)for(const q of grid.get(`${cx+dx},${cy+dy}`)??[])
      assert(Math.hypot(x-q.x,y-q.y)>=0.75-1e-8,'native collision');
    const key=`${cx},${cy}`;const cell=grid.get(key)??[];cell.push(p.position);grid.set(key,cell);i++;
  }
  assert.equal(i,before.length);
  const members=new Set<string>();
  for(const c of town.game.world.conversations.values())for(const member of c.participants.values()) {
    assert(town.game.world.players.has(member.playerId),'conversation player exists');
    assert(!members.has(member.playerId),'player belongs to only one conversation');
    members.add(member.playerId);
  }
  const clock=performance.now();const frame=JSON.stringify(town.snapshot(now,loopMs,ticks*steps*16));
  assert(frame.length>0);const snapshotMs=performance.now()-clock;
  const m=town.metrics;
  const row=`original,${i},${sample},${ticks*steps},${ticks*steps*16},${loopMs.toFixed(3)},${packMs.toFixed(3)},${snapshotMs.toFixed(3)},${moved},${m.decisions},${m.destinations},${m.activities},${m.invitations},${m.messages},${m.memories},${m.pathfinds},${town.historyBytes}\n`;
  process.stderr.write(row);if(sample>=0)appendFileSync(output,row);
}
