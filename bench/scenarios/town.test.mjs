import test from 'node:test';
import assert from 'node:assert/strict';
import { makeTown } from './town.mjs';
for (const n of [1, 32, 1000, 10000]) test(`town fixture: ${n} residents`, () => {
  const f=makeTown(n),m=f.map;
  assert.equal(f.players.length,n);
  assert.equal(new Set(f.players.map(p=>`${p.x},${p.y}`)).size,n);
  for(const p of f.players) {
    assert(m.objectTiles.every(l=>l[p.x/1000][p.y/1000]===-1));
    for(let i=0;i<p.path.length;i++) {
      const [x,y]=p.path[i];assert(m.objectTiles.every(l=>l[x/1000][y/1000]===-1));
      if(i) assert.equal(Math.abs(x-p.path[i-1][0])+Math.abs(y-p.path[i-1][1]),1000);
    }
  }
  if(n>=32) {
    assert(f.conversations.some(c=>c.status===1));assert(f.conversations.some(c=>c.status===2));assert(f.conversations.some(c=>c.status===3));
    assert.equal(new Set(f.players.map(p=>p.activity)).size,4);
    const directions=new Set(f.players.flatMap(p=>p.path.slice(1).map((v,i)=>`${v[0]-p.path[i][0]},${v[1]-p.path[i][1]}`)));
    assert.equal(directions.size,4);
    assert(f.density<0.5);
  }
});
test('seed reproduces fixture; changing it changes spawn locations',()=>{
  assert.deepEqual(makeTown(32,7),makeTown(32,7));
  assert.notDeepEqual(makeTown(32,7).players,makeTown(32,8).players);
});
