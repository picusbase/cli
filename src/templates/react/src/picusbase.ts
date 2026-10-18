// Picusbase client for this app: OAuth PKCE login, entity records (console-api) and
// Picus Functions (dispatcher, compute-audience token). Config comes from /runtime-config.json,
// served by the platform container in production and by vite.config.ts in `npm run dev`.

export type RuntimeConfig = {
  pinned_project_ref: string;
  pinned_app_code: string;
  pinned_oauth_client_id?: string | null;
  console_api_url: string;
  auth_api_url: string;
  functions_url: string;
  realtime_url?: string | null;
};

export type EntityRecord<T = Record<string, unknown>> = {
  id: string;
  entity_code: string;
  data: T;
  created_at: string | null;
  updated_at: string | null;
};

export type User = { id: string; email?: string };

type TokenResponse = { access_token: string; refresh_token?: string; expires_in?: number };
type Session = { access_token: string; refresh_token?: string; expires_at: number; client_id: string };

// Platform-wide client, used when the deployment has no project-specific OAuth client.
const SYSTEM_CLIENT_ID = 'ca951fce-0229-4207-88e9-f5c5a32bf9e7';
const SESSION_KEY = 'picus_session';
const PKCE_KEY = 'picus_pkce';

let configPromise: Promise<RuntimeConfig> | undefined;
let callbackPromise: Promise<boolean> | undefined;
let computeToken: { token: string; expires_at: number } | undefined;

export function getConfig(): Promise<RuntimeConfig> {
  configPromise ??= fetch('/runtime-config.json').then(async (res) => {
    if (!res.ok) throw new Error(`runtime-config.json: HTTP ${res.status}`);
    return (await res.json()) as RuntimeConfig;
  });
  return configPromise;
}

function base64Url(bytes: Uint8Array): string {
  let binary = '';
  bytes.forEach((b) => (binary += String.fromCharCode(b)));
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

function randomString(): string {
  return base64Url(crypto.getRandomValues(new Uint8Array(32)));
}

async function pkceChallenge(verifier: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier));
  return base64Url(new Uint8Array(digest));
}

function readSession(): Session | null {
  try {
    return JSON.parse(localStorage.getItem(SESSION_KEY) ?? 'null') as Session | null;
  } catch {
    return null;
  }
}

function saveSession(token: TokenResponse, clientId: string) {
  const session: Session = {
    access_token: token.access_token,
    refresh_token: token.refresh_token,
    expires_at: Date.now() + (token.expires_in ?? 900) * 1000,
    client_id: clientId,
  };
  localStorage.setItem(SESSION_KEY, JSON.stringify(session));
}

async function requestToken(body: Record<string, string>): Promise<TokenResponse> {
  const cfg = await getConfig();
  const res = await fetch(`${cfg.auth_api_url}/auth/oauth/token`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`token: HTTP ${res.status} ${await res.text()}`);
  return (await res.json()) as TokenResponse;
}

/** Redirects to the Picus login page; the browser comes back to /callback. */
export async function login(): Promise<void> {
  const cfg = await getConfig();
  const clientId = cfg.pinned_oauth_client_id || SYSTEM_CLIENT_ID;
  const verifier = randomString();
  const state = randomString();
  sessionStorage.setItem(PKCE_KEY, JSON.stringify({ verifier, state, clientId }));
  const params = new URLSearchParams({
    response_type: 'code',
    client_id: clientId,
    redirect_uri: `${location.origin}/callback`,
    scope: 'openid profile email',
    code_challenge: await pkceChallenge(verifier),
    code_challenge_method: 'S256',
    state,
  });
  location.assign(`${cfg.auth_api_url}/auth/oauth/authorize?${params}`);
}

async function exchangeCallback(): Promise<boolean> {
  if (location.pathname !== '/callback') return false;
  const params = new URLSearchParams(location.search);
  const pending = JSON.parse(sessionStorage.getItem(PKCE_KEY) ?? 'null') as
    | { verifier: string; state: string; clientId: string }
    | null;
  sessionStorage.removeItem(PKCE_KEY);
  history.replaceState(null, '', '/');
  const code = params.get('code');
  if (!pending || !code || params.get('state') !== pending.state) {
    throw new Error(params.get('error_description') ?? 'Invalid login callback');
  }
  const token = await requestToken({
    grant_type: 'authorization_code',
    code,
    client_id: pending.clientId,
    redirect_uri: `${location.origin}/callback`,
    code_verifier: pending.verifier,
  });
  saveSession(token, pending.clientId);
  return true;
}

/** Call once on startup: finishes the login when the page is /callback. */
export function completeLogin(): Promise<boolean> {
  callbackPromise ??= exchangeCallback();
  return callbackPromise;
}

export function logout() {
  localStorage.removeItem(SESSION_KEY);
  computeToken = undefined;
}

export function currentUser(): User | null {
  const session = readSession();
  if (!session) return null;
  try {
    const payload = JSON.parse(atob(session.access_token.split('.')[1].replace(/-/g, '+').replace(/_/g, '/')));
    return { id: payload.sub, email: payload.email };
  } catch {
    return null;
  }
}

async function accessToken(): Promise<string | null> {
  const session = readSession();
  if (!session) return null;
  if (Date.now() < session.expires_at - 30_000) return session.access_token;
  if (!session.refresh_token) {
    logout();
    return null;
  }
  try {
    const token = await requestToken({
      grant_type: 'refresh_token',
      refresh_token: session.refresh_token,
      client_id: session.client_id,
    });
    saveSession({ ...token, refresh_token: token.refresh_token ?? session.refresh_token }, session.client_id);
    return token.access_token;
  } catch {
    logout();
    return null;
  }
}

async function consoleApi<T>(method: string, path: string, body?: unknown): Promise<T> {
  const cfg = await getConfig();
  const token = await accessToken();
  const headers: Record<string, string> = {};
  if (body !== undefined) headers['content-type'] = 'application/json';
  if (token) headers.authorization = `Bearer ${token}`;
  const res = await fetch(`${cfg.console_api_url}/platform/projects/${cfg.pinned_project_ref}${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`${method} ${path}: HTTP ${res.status} ${await res.text()}`);
  return (await res.json()) as T;
}

/** Entity records. Reads may work anonymously; writes need a logged-in user with permission. */
export const records = {
  list: <T = Record<string, unknown>>(entity: string, opts: { page?: number; limit?: number } = {}) =>
    consoleApi<EntityRecord<T>[]>('GET', `/entities/${entity}/records?page=${opts.page ?? 1}&limit=${opts.limit ?? 20}`),
  get: <T = Record<string, unknown>>(entity: string, id: string) =>
    consoleApi<EntityRecord<T>>('GET', `/entities/${entity}/records/${encodeURIComponent(id)}`),
  create: <T = Record<string, unknown>>(entity: string, data: Partial<T>) =>
    consoleApi<EntityRecord<T>>('POST', `/entities/${entity}/records`, { data }),
  update: <T = Record<string, unknown>>(entity: string, id: string, data: Partial<T>) =>
    consoleApi<EntityRecord<T>>('PATCH', `/entities/${entity}/records/${encodeURIComponent(id)}`, { data }),
  remove: (entity: string, id: string) =>
    consoleApi<{ deleted: boolean; id: string }>('DELETE', `/entities/${entity}/records/${encodeURIComponent(id)}`),
};

async function getComputeToken(): Promise<string | null> {
  const token = await accessToken();
  if (!token) return null;
  if (computeToken && Date.now() < computeToken.expires_at - 30_000) return computeToken.token;
  const cfg = await getConfig();
  const res = await fetch(`${cfg.auth_api_url}/auth/token/exchange`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', authorization: `Bearer ${token}`, 'x-project-ref': cfg.pinned_project_ref },
    body: JSON.stringify({ target_project_ref: cfg.pinned_project_ref, target_audience: 'compute' }),
  });
  if (!res.ok) throw new Error(`compute token: HTTP ${res.status} ${await res.text()}`);
  const body = (await res.json()) as TokenResponse;
  computeToken = { token: body.access_token, expires_at: Date.now() + (body.expires_in ?? 300) * 1000 };
  return computeToken.token;
}

/** Calls any route bound to a Picus Function (see `picusbase route list`). */
export async function requestFunction<T = unknown>(method: string, path: string, body?: unknown): Promise<T> {
  const cfg = await getConfig();
  const token = await getComputeToken();
  const headers: Record<string, string> = { 'x-project-ref': cfg.pinned_project_ref };
  if (body !== undefined) headers['content-type'] = 'application/json';
  if (token) headers.authorization = `Bearer ${token}`;
  const res = await fetch(`${cfg.functions_url}${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`${method} ${path}: HTTP ${res.status} ${await res.text()}`);
  return (await res.json()) as T;
}

/** `POST /functions/<key>/execute` — same route `picusbase function run` uses. */
export function callFunction<T = unknown>(functionKey: string, params: unknown = {}): Promise<T> {
  return requestFunction<T>('POST', `/functions/${functionKey}/execute`, params);
}
