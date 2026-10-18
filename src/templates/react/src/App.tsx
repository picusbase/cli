import { useEffect, useState } from 'react';
import {
  callFunction,
  completeLogin,
  currentUser,
  getConfig,
  login,
  logout,
  records,
  type EntityRecord,
  type RuntimeConfig,
} from './picusbase';

export default function App() {
  const [config, setConfig] = useState<RuntimeConfig>();
  const [user, setUser] = useState(currentUser());
  const [error, setError] = useState<string>();

  useEffect(() => {
    getConfig().then(setConfig).catch((e) => setError(String(e)));
    completeLogin()
      .then(() => setUser(currentUser()))
      .catch((e) => setError(String(e)));
  }, []);

  return (
    <main>
      <header>
        <h1>{{APP_NAME}}</h1>
        <span className="muted">{config?.pinned_project_ref}</span>
        <div className="spacer" />
        {user ? (
          <>
            <span>{user.email ?? user.id}</span>
            <button onClick={() => { logout(); setUser(null); }}>Đăng xuất</button>
          </>
        ) : (
          <button onClick={() => login().catch((e) => setError(String(e)))}>Đăng nhập</button>
        )}
      </header>
      {error && <p className="error">{error}</p>}
      <FunctionPanel />
      <RecordsPanel />
    </main>
  );
}

function FunctionPanel() {
  const [functionKey, setFunctionKey] = useState('');
  const [params, setParams] = useState('{}');
  const [result, setResult] = useState<string>();

  const run = async () => {
    try {
      setResult(JSON.stringify(await callFunction(functionKey, JSON.parse(params)), null, 2));
    } catch (e) {
      setResult(String(e));
    }
  };

  return (
    <section>
      <h2>Gọi function</h2>
      <div className="row">
        <input placeholder="function key" value={functionKey} onChange={(e) => setFunctionKey(e.target.value)} />
        <input value={params} onChange={(e) => setParams(e.target.value)} />
        <button disabled={!functionKey} onClick={run}>Chạy</button>
      </div>
      {result && <pre>{result}</pre>}
    </section>
  );
}

function RecordsPanel() {
  const [entity, setEntity] = useState('');
  const [rows, setRows] = useState<EntityRecord[]>([]);
  const [error, setError] = useState<string>();

  const load = async () => {
    try {
      setError(undefined);
      setRows(await records.list(entity));
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <section>
      <h2>Bản ghi entity</h2>
      <div className="row">
        <input placeholder="entity code" value={entity} onChange={(e) => setEntity(e.target.value)} />
        <button disabled={!entity} onClick={load}>Tải</button>
      </div>
      {error && <p className="error">{error}</p>}
      {rows.length > 0 && (
        <table>
          <thead>
            <tr><th>id</th><th>data</th></tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.id}>
                <td>{row.id}</td>
                <td><code>{JSON.stringify(row.data)}</code></td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
