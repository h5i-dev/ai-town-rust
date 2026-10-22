import { useMemo, useState } from 'react';
import { ensureSession, forgetSession, rpc, storedUser, useTown } from './api';
import { Town } from './components/Town';
import { ST_INVITED, ST_TALKING, ST_WALKING, type Player, type World } from './types';

export default function App() {
  const { world, map, now, connected } = useTown();
  const [user, setUser] = useState<number | null>(storedUser());
  const [selected, setSelected] = useState<number | null>(null);
  const [name, setName] = useState('Ada');
  const [draft, setDraft] = useState('');
  const [error, setError] = useState('');
  const [music, setMusic] = useState(false);

  const demo = map?.demo ?? false;
  const me = world?.players.find((p) => p.owner === user) ?? null;
  const other = world?.players.find((p) => p.id === selected) ?? null;
  const mine = useMemo(() => conversationOf(world?.conversations ?? [], me?.id), [world, me?.id]);
  const myMember = mine?.members.find((m) => m.player === me?.id);

  async function run(cmd: Record<string, unknown>) {
    setError('');
    try {
      await rpc(cmd);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'request failed');
    }
  }

  async function join() {
    setError('');
    try {
      const id = await ensureSession();
      setUser(id);
      await rpc({ cmd: 'join', name, character: 1 });
    } catch (err) {
      setError(err instanceof Error ? err.message : 'join failed');
    }
  }

  async function leave() {
    await run({ cmd: 'leave' });
    forgetSession();
    setUser(null);
  }

  const messages = (world?.messages ?? []).filter((m) => m.conversation === mine?.id);

  return (
    <main className="game-background font-body flex min-h-screen flex-col text-white">
      <header className="flex items-end justify-between gap-4 px-6 pt-4">
        <div>
          <h1 className="game-title text-5xl sm:text-7xl">AI Town</h1>
          <p className="shadow-solid mt-1 text-lg">{demo ? `${map?.demoEngine === 'original' ? 'Original AI Town' : 'Rust'} movement demo · no LLM · no database` : 'A virtual town where AI characters live, chat and socialize.'}</p>
        </div>
        <p className="text-sm text-clay-100">{world?.players.length.toLocaleString() ?? '—'} players · {connected ? 'live' : 'reconnecting'}</p>
      </header>
      <div className="grid min-h-0 flex-1 grid-cols-1 gap-4 p-4 lg:grid-cols-[1fr_22rem]">
        <div className="game-frame relative h-[70vh] min-h-[28rem] overflow-hidden bg-brown-900">
          {world && map ? (
            <Town
              map={map}
              world={world}
              now={now}
              meId={me?.id ?? null}
              onMove={(x, y) => void run({ cmd: 'move_to', x, y })}
              onSelect={setSelected}
            />
          ) : (
            <p className="p-8 text-xl">Loading the town…</p>
          )}
        </div>
        <aside className="box flex flex-col gap-3 bg-brown-800 p-4 text-brown-200">
          {error && <p className="text-red-300">{error}</p>}
          {world?.demo && <p>Original engine: {world.demo.stepMs} ms simulated in {Math.round(world.demo.tickMs).toLocaleString()} ms. Positions update as computation finishes.</p>}
          {demo && <p>Drag to pan, scroll to zoom. Select a character to inspect it. Players walk back and forth using the selected movement engine.</p>}
          {!demo && !me && (
            <form
              className="flex flex-col gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                void join();
              }}
            >
              <label className="text-sm">
                Name
                <input
                  className="mt-1 w-full bg-brown-900 px-2 py-1 text-white"
                  value={name}
                  maxLength={32}
                  onChange={(e) => setName(e.target.value)}
                />
              </label>
              <button className="button" type="submit">
                <span>Join</span>
              </button>
            </form>
          )}
          {me && (
            <div className="flex gap-2">
              <button className="button" type="button" onClick={() => void run({ cmd: 'stop' })}>
                <span>Stop</span>
              </button>
              <button className="button" type="button" onClick={() => void leave()}>
                <span>Leave</span>
              </button>
            </div>
          )}
          {myMember?.status === ST_WALKING && <p>On the way over.</p>}
          {myMember?.status === ST_INVITED && <p>Someone invited you over.</p>}
          {mine && myMember && myMember.status !== ST_INVITED && (
            <button className="button" type="button" onClick={() => void run({ cmd: 'leave_conversation' })}>
              <span>Leave conversation</span>
            </button>
          )}
          {myMember?.status === ST_INVITED && (
            <div className="flex gap-2">
              <button className="button" type="button" onClick={() => void run({ cmd: 'accept_invite' })}>
                <span>Accept</span>
              </button>
              <button className="button" type="button" onClick={() => void run({ cmd: 'reject_invite' })}>
                <span>Decline</span>
              </button>
            </div>
          )}
          <PlayerCard
            player={other}
            world={world}
            me={me}
            talking={myMember?.status === ST_TALKING}
            onStart={() => other && void run({ cmd: 'start_conversation', invitee: other.id })}
          />
          {mine && (
            <div className="flex min-h-0 flex-1 flex-col gap-2">
              <div className="max-h-64 space-y-2 overflow-auto">
                {messages.map((m) => (
                  <p key={m.id} className={m.author === me?.id ? 'bubble-mine bubble' : 'bubble'}>
                    <span className="font-bold">{world?.players.find((p) => p.id === m.author)?.name ?? 'Someone'}: </span>
                    {m.text}
                  </p>
                ))}
              </div>
              {myMember?.status === ST_TALKING && (
                <form
                  className="flex gap-2"
                  onSubmit={(e) => {
                    e.preventDefault();
                    const text = draft.trim();
                    if (!text) return;
                    setDraft('');
                    void run({ cmd: 'say', text });
                  }}
                >
                  <input
                    className="w-full bg-brown-900 px-2 py-1 text-white"
                    value={draft}
                    maxLength={280}
                    placeholder="Say something"
                    onChange={(e) => setDraft(e.target.value)}
                  />
                  <button className="button" type="submit">
                    <span>Say</span>
                  </button>
                </form>
              )}
            </div>
          )}
          <button
            className="button mt-auto"
            type="button"
            onClick={() => {
              const audio = document.getElementById('town-music') as HTMLAudioElement | null;
              if (!audio) return;
              if (music) {
                audio.pause();
                setMusic(false);
              } else {
                void audio.play().then(() => setMusic(true)).catch(() => setError('music blocked'));
              }
            }}
          >
            <span>{music ? 'Mute' : 'Music'}</span>
          </button>
          <audio id="town-music" src="/ai-town/assets/background.mp3" loop />
        </aside>
      </div>
    </main>
  );
}

function conversationOf(conversations: { id: number; members: { player: number; status: number }[] }[], playerId: number | undefined) {
  if (playerId === undefined) return undefined;
  return conversations.find((c) => c.members.some((m) => m.player === playerId));
}

function PlayerCard({
  player,
  world,
  me,
  talking,
  onStart,
}: {
  player: Player | null;
  world: World | null;
  me: Player | null;
  talking: boolean;
  onStart: () => void;
}) {
  if (!player || !world) return <p className="text-clay-300">Click a character to look closer.</p>;
  const agent = world.agents.find((a) => a.player === player.id);
  return (
    <div>
      <h2 className="text-2xl text-white">{player.name}</h2>
      {agent && <p className="mt-2 text-sm leading-snug">{agent.identity}</p>}
      {agent && <p className="mt-2 text-sm text-clay-300">{agent.plan}</p>}
      {me && player.id !== me.id && !talking && (
        <button className="button mt-3" type="button" onClick={onStart}>
          <span>Start conversation</span>
        </button>
      )}

    </div>
  );
}
