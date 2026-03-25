const RELAY_URL = process.env.SYNAPSE_RELAY_URL ?? "http://127.0.0.1:7779";

export interface RelayMessage {
  seq: number;
  type: "dialogue" | "work";
  text: string | null;
  payload: unknown | null;
  received_at: number;
}

interface PollResp  { messages: RelayMessage[]; next_seq: number }
interface WaitResp  { messages: RelayMessage[]; next_seq: number; timed_out: boolean }

const nextSeq = new Map<string, number>();

function getSeq(channel: string): number { return nextSeq.get(channel) ?? 0; }
function setSeq(channel: string, seq: number) { nextSeq.set(channel, seq); }

async function relayFetch(path: string, init?: RequestInit): Promise<unknown> {
  const res = await fetch(`${RELAY_URL}${path}`, init);
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: res.statusText }));
    throw new Error((body as { error?: string }).error ?? res.statusText);
  }
  return res.json();
}

export async function checkRelay(): Promise<boolean> {
  try { await relayFetch("/status"); return true; }
  catch { return false; }
}

export async function send(channel: string, text: string): Promise<void> {
  await relayFetch("/send", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ channel, text }),
  });
}

export async function sendWork(channel: string, payload: unknown): Promise<void> {
  await relayFetch("/send_work", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ channel, payload }),
  });
}

export async function poll(channel: string): Promise<RelayMessage[]> {
  const since = getSeq(channel);
  const resp = await relayFetch(`/poll?channel=${encodeURIComponent(channel)}&since=${since}`) as PollResp;
  setSeq(channel, resp.next_seq);
  return resp.messages;
}

export async function wait(
  channel: string, min = 1, timeoutSecs = 30
): Promise<{ messages: RelayMessage[]; timedOut: boolean }> {
  const since = getSeq(channel);
  const resp = await relayFetch(
    `/wait?channel=${encodeURIComponent(channel)}&since=${since}&min=${min}&timeout=${timeoutSecs * 1000}`
  ) as WaitResp;
  setSeq(channel, resp.next_seq);
  return { messages: resp.messages, timedOut: resp.timed_out };
}

export async function subscribe(channel: string): Promise<void> {
  await relayFetch("/subscribe", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ channel }),
  });
}

export async function leave(channel: string): Promise<void> {
  await relayFetch("/leave", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ channel }),
  });
}

export async function listChannels(): Promise<string[]> {
  const resp = await relayFetch("/channels") as { channels: string[] };
  return resp.channels;
}

export async function listUsers(channel: string): Promise<string[]> {
  const resp = await relayFetch(`/users?channel=${encodeURIComponent(channel)}`) as { users: string[] };
  return resp.users;
}
