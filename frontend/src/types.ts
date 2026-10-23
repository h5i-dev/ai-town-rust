export type PathNode = {
  seq: number;
  x: number;
  y: number;
  faceDx: number;
  faceDy: number;
  t: number;
};

export type Player = {
  id: number;
  owner: number;
  human: boolean;
  name: string;
  character: number;
  x: number;
  y: number;
  faceDx: number;
  faceDy: number;
  speed: number;
  destX: number;
  destY: number;
  pf: number;
  activity: number;
  activityUntil: number;
  path: PathNode[];
};

export type Member = { player: number; status: number; invited: number; started: number };

export type Conversation = {
  id: number;
  creator: number;
  created: number;
  typingPlayer: number;
  typingSince: number;
  lastAuthor: number;
  lastAt: number;
  numMessages: number;
  members: Member[];
};

export type ChatMessage = {
  id: number;
  conversation: number;
  author: number;
  ts: number;
  text: string;
};

export type Agent = {
  id: number;
  player: number;
  owner: number;
  name: string;
  character: number;
  identity: string;
  plan: string;
  op: number;
  toRemember: number;
};

export type World = {
  demo?: { engine: 'rust' | 'original'; scenario?: 'town'; tickMs: number; stepMs: number; districts?: number; density?: number;
    metrics?: { decisions: number; destinations: number; activities: number; invitations: number; messages: number; memories: number; pathfinds: number } };
  nowMs: number;
  width: number;
  height: number;
  frozen: boolean;
  players: Player[];
  conversations: Conversation[];
  messages: ChatMessage[];
  agents: Agent[];
};

export type TownMap = {
  demo?: boolean;
  demoEngine?: 'rust' | 'original';
  demoScenario?: 'town';
  tileSetUrl: string;
  tileSetDimX: number;
  tileSetDimY: number;
  tileDim: number;
  width: number;
  height: number;
  bgTiles: number[][][];
  objectTiles: number[][][];
  animatedSprites: {
    x: number;
    y: number;
    w: number;
    h: number;
    layer: number;
    sheet: string;
    animation: string;
  }[];
};

export const ST_INVITED = 1;
export const ST_WALKING = 2;
export const ST_TALKING = 3;
