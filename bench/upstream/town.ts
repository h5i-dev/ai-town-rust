import { Game } from 'upstream-game';
import { readFileSync } from 'node:fs';
import { hash, START } from '../scenarios/policy.mjs';

const number = (id: string) => Number(id.slice(2));
const activityNames = ['', 'reading a book', 'gardening', 'daydreaming'];
const emojis = ['', '📖', '🌱', '✨'];
export class Town {
  fixture: any;
  game: Game;
  messages: any[] = [];
  memories: any[] = [];
  metrics = { decisions: 0, destinations: 0, activities: 0, invitations: 0, messages: 0, memories: 0, pathfinds: 0 };
  constructor(path: string) {
    this.fixture = JSON.parse(readFileSync(path, 'utf8'));
    const f = this.fixture;
    const players = f.players.map((p: any) => {
      const path = p.path.map(([x,y]: number[], j: number) => {
        const next = p.path[Math.min(j+1,p.path.length-1)], prev = p.path[Math.max(0,j-1)];
        const dx = Math.sign(j+1<p.path.length ? next[0]-x : x-prev[0]);
        const dy = Math.sign(j+1<p.path.length ? next[1]-y : y-prev[1]);
        return [x/1000,y/1000,dx,dy,START + j * 4000/3];
      });
      return { id: `p:${p.id}`, position: {x:p.x/1000,y:p.y/1000}, facing:{dx:1,dy:0}, speed:0,lastInput:START,
        activity: p.activity ? {description:activityNames[p.activity],emoji:emojis[p.activity],until:p.until} : undefined,
        pathfinding: path.length > 1 ? {destination:{x:path.at(-1)[0],y:path.at(-1)[1]},started:START,state:{kind:'moving',path}} : undefined };
    });
    const conversations = f.conversations.map((c: any) => ({id:`c:${c.id}`,creator:`p:${c.a}`,created:START-5000,
      numMessages:c.messages,lastMessage:c.messages ? {author:`p:${c.b}`,timestamp:START-3000}:undefined,
      participants:[{playerId:`p:${c.a}`,invited:START-1000,status:c.status===3?{kind:'participating',started:START-5000}:{kind:'walkingOver'}},
        {playerId:`p:${c.b}`,invited:START-1000,status:c.status===3?{kind:'participating',started:START-5000}:c.status===1?{kind:'invited'}:{kind:'walkingOver'}}]}));
    this.game = new Game({_id:'town',currentTime:START} as any,'town' as any,{
      world:{nextId:f.players.length*4+1,players,conversations,
        agents:players.map((p: any) => ({id:`a:${number(p.id)+f.players.length}`,playerId:p.id,lastInviteAttempt:START}))},
      worldMap:f.map,playerDescriptions:[],agentDescriptions:[],
    } as any);
    let seed = f.seed;
    // Scoped per process/worker; controls native invitation/backoff randomness.
    Math.random = () => {seed=(Math.imul(seed,1664525)+1013904223)>>>0; return seed/2**32;};
  }
  destination(id: number, now: number, x: number, y: number) {
    const m = this.fixture.map, salt = hash(id + this.fixture.seed + Math.floor((now-START)/15000));
    for (let k=0;k<512;k++) {
      const h=hash(salt+k), tx=Math.floor(x)+(h%25)-12, ty=Math.floor(y)+((h>>>8)%25)-12;
      // Avoid epsilon-length native paths whose timestamps can be identical.
      if (tx>=0&&ty>=0&&tx<m.width&&ty<m.height&&Math.hypot(tx-x,ty-y)>=0.5&&m.objectTiles.every((l:any)=>l[tx][ty]===-1)) return {x:tx,y:ty};
    }
    return {x:Math.floor(x),y:Math.floor(y)};
  }
  drain(now: number) {
    const game = this.game, operations = game.pendingOperations.splice(0);
    for (const {name,args} of operations) {
      const agent = game.world.agents.get(args.agentId ?? args.agent?.id);
      if (!agent || agent.inProgressOperation?.operationId !== args.operationId) continue;
      const player = game.world.players.get(agent.playerId)!;
      if (name==='agentDoSomething') {
        this.metrics.decisions++;
        const id=number(player.id), salt=hash(id+this.fixture.seed+Math.floor((now-START)/15000));
        const response:any={agentId:agent.id,operationId:args.operationId};
        const inConversation=(p:any)=>[...game.world.conversations.values()].some(c=>c.participants.has(p.id));
        if (!inConversation(player)) {
          if (salt%4===0) {
            const target=[...game.world.players.values()].find(p=>p.id!==player.id&&!inConversation(p)
              &&Math.hypot(p.position.x-player.position.x,p.position.y-player.position.y)<12);
            if(target) {response.invitee=target.id;this.metrics.invitations++;}
          }
          if (!response.invitee&&!player.pathfinding) {
            if(salt%4===1) {response.activity={description:activityNames[1+salt%3],emoji:emojis[1+salt%3],until:now+60000};this.metrics.activities++;}
            else {response.destination=this.destination(id,now,player.position.x,player.position.y);this.metrics.destinations++;}
          }
        }
        game.handleInput(now,'finishDoSomething',response);
      } else if (name==='agentGenerateMessage') {
        const c=game.world.conversations.get(args.conversationId);
        if(c) {
          this.messages.push({id:this.metrics.messages+1,conversation:number(c.id),author:number(player.id),ts:now,
            text:`${activityNames[1+number(player.id)%3]} sounds nice. See you around town!`});
          this.metrics.messages++;
          game.handleInput(now,'agentFinishSendingMessage',{agentId:agent.id,operationId:args.operationId,
            conversationId:c.id,timestamp:now,leaveConversation:args.type==='leave'});
        } else delete agent.inProgressOperation;
      } else if(name==='agentRememberConversation') {
        this.metrics.memories++; this.memories.push({player:number(player.id),conversation:number(args.conversationId),summary:'Talked with a neighbor about life in town.'});
        game.handleInput(now,'finishRememberConversation',{agentId:agent.id,operationId:args.operationId});
      } else throw Error(`Unhandled operation ${name}`);
    }
  }
  step(now: number, ticks: number) {
    const started=performance.now();
    this.game.beginStep(now+16);
    for(let i=1;i<=ticks;i++) this.game.tick(now+i*16);
    this.metrics.pathfinds+=this.game.numPathfinds;
    this.drain(now+ticks*16);
    const tickMs=performance.now()-started;
    const packStarted=performance.now(); this.game.takeDiff();
    return {tickMs,packMs:performance.now()-packStarted};
  }
  snapshot(now: number, tickMs=0, stepMs=0) {
    const game=this.game;
    return {nowMs:now,width:game.worldMap.width,height:game.worldMap.height,frozen:false,
      demo:{engine:'original',scenario:'town',tickMs,stepMs,metrics:this.metrics,districts:this.fixture.districts,density:this.fixture.density},
      players:[...game.world.players.values()].map(p=>({id:number(p.id),owner:0,human:false,name:`Resident ${number(p.id)}`,character:(number(p.id)-1)%8,
        x:Math.round(p.position.x*1000),y:Math.round(p.position.y*1000),faceDx:Math.round((p.facing.dx+1)*1000),faceDy:Math.round((p.facing.dy+1)*1000),
        speed:Math.round(p.speed*1000),destX:Math.round((p.pathfinding?.destination.x??p.position.x)*1000),destY:Math.round((p.pathfinding?.destination.y??p.position.y)*1000),
        pf:p.pathfinding?({needsPath:1,waiting:2,moving:3}[p.pathfinding.state.kind]):0,
        activity:p.activity?Math.max(0,activityNames.indexOf(p.activity.description)):0,activityUntil:p.activity?.until??0,path:[]})),
      agents:[...game.world.agents.values()].map(a=>({id:number(a.id),player:number(a.playerId),owner:0,name:`Resident ${number(a.playerId)}`,character:(number(a.playerId)-1)%8,
        identity:'A resident of the town.',plan:'Walk around, enjoy an activity, and talk with neighbors.',op:a.inProgressOperation?1:0,toRemember:a.toRemember?number(a.toRemember):0})),
      conversations:[...game.world.conversations.values()].map(c=>({id:number(c.id),creator:number(c.creator),created:c.created,
        typingPlayer:c.isTyping?number(c.isTyping.playerId):0,typingSince:c.isTyping?.since??0,lastAuthor:c.lastMessage?number(c.lastMessage.author):0,
        lastAt:c.lastMessage?.timestamp??0,numMessages:c.numMessages,members:[...c.participants.values()].map(m=>({player:number(m.playerId),invited:m.invited,
          status:{invited:1,walkingOver:2,participating:3}[m.status.kind],started:m.status.kind==='participating'?m.status.started:0}))})),
      messages:this.messages.slice(-100),
    };
  }
}
