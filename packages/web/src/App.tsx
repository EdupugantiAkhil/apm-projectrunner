import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from 'react'
import { ApiClient, ApiError, type AdapterRecord, type DeploymentDetail, type DeploymentSummary, type Operation, type OperationEvent, type RouteState, type SourceRecord } from './api'
import DeploymentWorkspace, { RoutingEditor } from './DeploymentWorkspace'
import DeploymentBuilder, { BlockLibrary } from './DeploymentBuilder'
import './App.css'

type View = 'deployments' | 'sources' | 'operations' | 'builder' | 'library'
const terminal = (status: Operation['status']) => ['succeeded', 'failed', 'cancelled'].includes(status)
const short = (value?: string | null) => value ? value.slice(0, 9) : 'unknown'
const dirtyText = (source: SourceRecord) => {
  const changes = source.inspection.changes
  return changes ? `${changes.staged} staged, ${changes.unstaged} unstaged, ${changes.untracked} untracked` : 'dirty details unavailable'
}

export default function App({ client = new ApiClient() }: { client?: ApiClient }) {
  const [view, setView] = useState<View>('deployments')
  const [deployments, setDeployments] = useState<DeploymentSummary[]>([])
  const [selected, setSelected] = useState('')
  const [detail, setDetail] = useState<DeploymentDetail | null>(null)
  const [routes, setRoutes] = useState<RouteState | null>(null)
  const [sources, setSources] = useState<SourceRecord[]>([])
  const [adapters, setAdapters] = useState<AdapterRecord[]>([])
  const [operations, setOperations] = useState<Operation[]>([])
  const [events, setEvents] = useState<OperationEvent[]>([])
  const [drawerOpen, setDrawerOpen] = useState(true)
  const [filter, setFilter] = useState('')
  const [notice, setNotice] = useState('Ready')
  const [error, setError] = useState('')
  const subscriptions = useRef<Map<string, { close(): void }>>(new Map())

  const report = (value: unknown) => setError(value instanceof ApiError ? `${value.code}: ${value.message}` : String(value))
  const loadDeployments = async () => {
    try {
      const response = await client.deployments()
      setDeployments(response.deployments)
      setSelected((current) => current || response.deployments[0]?.name || '')
    } catch (value) { report(value) }
  }
  const loadSources = async () => { try { setSources(await client.sources()) } catch (value) { report(value) } }
  const loadSelected = async () => { if (!selected) return; const [nextDetail, nextRoutes] = await Promise.all([client.deployment(selected), client.routes(selected)]); setDetail(nextDetail); setRoutes(nextRoutes) }

  useEffect(() => { void loadDeployments(); void loadSources(); void client.adapters().then(setAdapters).catch(report) }, [])
  useEffect(() => {
    if (!selected) { setDetail(null); setRoutes(null); return }
    void loadSelected().catch(report)
  }, [selected])
  useEffect(() => () => { for (const subscription of subscriptions.current.values()) subscription.close() }, [])

  const observe = (started: Operation) => {
    setOperations((current) => [started, ...current.filter((item) => item.id !== started.id)])
    setNotice(`${started.kind} ${started.status}`)
    const subscription = client.subscribe(started.id, (event) => {
      setEvents((current) => [...current, event])
      if (event.kind !== 'log') setNotice(`${event.kind} transition`)
    }, () => setNotice(`event stream reconnecting for ${started.id}`))
    subscriptions.current.set(started.id, subscription)
    void client.pollOperation(started.id).then((finished) => {
      setOperations((current) => current.map((item) => item.id === finished.id ? finished : item))
      setNotice(`${finished.kind} ${finished.status}`)
      subscription.close()
      subscriptions.current.delete(finished.id)
      void loadDeployments()
    }).catch(report)
  }
  const runCommand = async (kind: 'validate' | 'plan' | 'status' | 'logs' | 'apply' | 'down' | 'cleanup', target?: string) => {
    if (!selected) return
    if (kind === 'apply' && Object.values(detail?.sourceIdentities ?? {}).some((identity) => identity.dirty) && !window.confirm('One or more source worktrees are modified. Continue with Up?')) {
      setNotice('up cancelled: modified worktrees were not acknowledged')
      return
    }
    if (kind === 'down' || kind === 'cleanup') {
      const typed = window.prompt(`Type ${selected} to confirm ${kind}`)
      if (typed !== selected) { setNotice(`${kind} cancelled: confirmation did not match`); return }
    }
    const bundle = `.switchyard/generated/${selected}/resolved-deployment.yaml`
    try { observe(await client.command(kind, bundle, { ...(kind === 'cleanup' ? { confirmed: true } : {}), ...(kind === 'logs' && target ? { target } : {}) })); setView('operations') } catch (value) { report(value) }
  }
  const navKeys = (event: KeyboardEvent<HTMLElement>) => {
    if (!['ArrowLeft', 'ArrowRight'].includes(event.key)) return
    const views: View[] = ['deployments', 'sources', 'operations', 'library']
    const offset = event.key === 'ArrowRight' ? 1 : -1
    setView(views[(views.indexOf(view) + offset + views.length) % views.length])
    event.preventDefault()
  }
  const visibleEvents = events.filter((event) => !filter || operations.find((operation) => operation.id === event.operationId)?.deployment === filter)

  return <div className="app-shell">
    <aside className="rail" aria-label="Deployment rail">
      <div className="brand">SWITCHYARD <span>LOCAL</span></div>
      <nav aria-label="Main views" onKeyDown={navKeys}>
        {(['deployments', 'sources', 'operations', 'library'] as View[]).map((item) => <button key={item} aria-current={view === item ? 'page' : undefined} onClick={() => setView(item)}>{item === 'library' ? 'block library' : item}</button>)}
      </nav>
      <h2>Deployments</h2>
      <div className="deployment-list">
        {deployments.length === 0 && <p className="muted">No deployments applied</p>}
        {deployments.map((deployment) => {
          const status = deployment.lastOperation?.status ?? 'unknown'
          return <button className="deployment-button" aria-pressed={selected === deployment.name} key={deployment.name} onClick={() => { setSelected(deployment.name); setView('deployments') }}>
            <span className={`status-dot status-${status}`} aria-hidden="true" />
            <span><strong>{deployment.name}</strong><small>{status}</small></span>
          </button>
        })}
      </div>
      <button className="new-deployment" onClick={() => setView('builder')}>+ New deployment</button>
    </aside>
    <main className="canvas" id="main-content">
      {error && <div className="error" role="alert"><span>{error}</span><button aria-label="Dismiss error" onClick={() => setError('')}>×</button></div>}
      {view === 'deployments' && <DeploymentView client={client} detail={detail} routes={routes} onCommand={runCommand} observe={observe} refresh={async () => { await loadSelected(); await loadDeployments() }} report={report} />}
      {view === 'sources' && <SourcesView client={client} sources={sources} reload={loadSources} report={report} />}
      {view === 'operations' && <OperationsView operations={operations} onCancel={async (id) => { if (!window.confirm('Cancel this running operation?')) return; try { const cancelled = await client.cancel(id); setOperations((current) => current.map((item) => item.id === id ? cancelled : item)) } catch (value) { report(value) } }} />}
      {view === 'builder' && <DeploymentBuilder client={client} sources={sources} close={() => setView('deployments')} onOperation={observe} report={report} saved={async (name) => { await loadDeployments(); setSelected(name); setView('deployments'); setNotice(`Deployment ${name} saved; use Up when ready`) }} />}
      {view === 'library' && <BlockLibrary adapters={adapters} />}
    </main>
    <aside className="inspector" aria-label="Inspector">
      <h2>Inspector</h2>
      {detail ? <><p className="eyebrow">Deployment</p><h3>{detail.deployment}</h3><dl><dt>Definition</dt><dd className="mono">{short(detail.definitionHash)}</dd><dt>Resources</dt><dd className="mono">{short(detail.resourceHash)}</dd><dt>Drift</dt><dd>{detail.reconciliation.diagnostics.length ? `${detail.reconciliation.diagnostics.length} warnings` : 'Reconciled'}</dd></dl>
        <h3>Domains</h3>{detail.customDomains.length ? <ul>{detail.customDomains.map((domain) => <li className="mono" key={domain}>{domain}</li>)}</ul> : <p className="muted">None</p>}
        <h3>Bindings</h3><dl>{Object.entries(detail.bindings).map(([consumer, group]) => <div key={consumer}><dt className="mono">{consumer}</dt><dd>{group}</dd></div>)}</dl></> : <p className="muted">Select a deployment</p>}
    </aside>
    <section className={`event-drawer ${drawerOpen ? 'open' : ''}`} aria-label="Events and logs">
      <header><button aria-expanded={drawerOpen} onClick={() => setDrawerOpen((value) => !value)}>Events & logs {drawerOpen ? '▾' : '▴'}</button><label>Deployment <select value={filter} onChange={(event) => setFilter(event.target.value)}><option value="">All</option>{deployments.map((deployment) => <option key={deployment.name}>{deployment.name}</option>)}</select></label><button onClick={() => void navigator.clipboard?.writeText(visibleEvents.map(eventText).join('\n'))}>Copy plain text</button></header>
      {drawerOpen && <div className="log-lines" role="log">{visibleEvents.length ? visibleEvents.map((event) => <div key={`${event.operationId}-${event.id}`}><time>{new Date(event.timestamp).toLocaleTimeString()}</time> <b>{event.kind}</b> {eventText(event)}</div>) : <p>No events yet.</p>}</div>}
    </section>
    <div className="sr-only" aria-live="polite" aria-atomic="true">{notice}</div>
  </div>
}

function DeploymentView({ client, detail, routes, onCommand, observe, refresh, report }: { client: ApiClient; detail: DeploymentDetail | null; routes: RouteState | null; onCommand: (kind: 'validate' | 'plan' | 'status' | 'logs' | 'apply' | 'down' | 'cleanup', target?: string) => void; observe: (operation: Operation) => void; refresh: () => Promise<void>; report: (error: unknown) => void }) {
  if (!detail) return <section><h1>Deployments</h1><p>No applied deployment selected.</p></section>
  const instances = detail.snapshot?.spec?.instances ?? Object.keys(detail.sourceIdentities).map((name) => ({ name }))
  return <section><div className="title-row"><div><p className="eyebrow">Deployment</p><h1>{detail.deployment}</h1></div><span className="state-label">● {detail.reconciliation.diagnostics.length ? 'Needs attention' : 'Reconciled'}</span></div>
    <div className="command-bar" aria-label="Deployment commands"><button onClick={() => onCommand('validate')}>Validate</button><button onClick={() => onCommand('plan')}>Plan</button><button onClick={() => onCommand('status')}>Status</button><button onClick={() => onCommand('logs')}>Logs</button><button className="primary" onClick={() => onCommand('apply')}>Up</button><button className="danger" onClick={() => onCommand('down')}>Down</button><button className="danger" onClick={() => onCommand('cleanup')}>Cleanup</button></div>
    <h2>Instances</h2><div className="instance-grid">{instances.map((instance) => { const identity = detail.sourceIdentities[instance.name]; const resource = detail.resources.find((item) => item.labels['dev.switchyard.instance'] === instance.name || item.name.includes(instance.name)); return <article className="instance-card" key={instance.name}><header><h3>{instance.name}</h3><span>{resource?.state ?? 'state unknown'}</span><button aria-label={`Logs for ${instance.name}`} onClick={() => onCommand('logs', instance.name)}>Logs</button></header>{identity ? <dl><dt>Path</dt><dd className="mono">{identity.path}</dd><dt>Ref</dt><dd className="mono">{identity.ref ?? 'detached'}</dd><dt>Commit</dt><dd className="mono">{short(identity.commit)} {identity.dirty ? <span className="dirty">● modified</span> : 'clean'}</dd></dl> : <p>Source identity unavailable</p>}</article> })}</div>
    <DeploymentWorkspace client={client} detail={detail} routes={routes} onOperation={observe} refresh={refresh} report={report} />
    <h2>Active routes</h2>{routes?.bindings.length ? <table><thead><tr><th>Consumer</th><th>Router</th><th>Version</th><th>Status</th></tr></thead><tbody>{routes.bindings.map((route) => <tr key={`${route.router}-${route.binding}`}><td className="mono">{route.binding}</td><td className="mono">{route.router}</td><td className="mono">v{route.currentVersion ?? route.desiredVersion ?? '—'}</td><td>{route.status}{route.lastErrorCode ? ` · ${route.lastErrorCode}` : ''}</td></tr>)}</tbody></table> : <p className="muted">No active route versions recorded.</p>}
    <RoutingEditor client={client} deployment={detail.deployment} onSaved={refresh} onOperation={observe} report={report} />
  </section>
}

function SourcesView({ client, sources, reload, report }: { client: ApiClient; sources: SourceRecord[]; reload: () => Promise<void>; report: (error: unknown) => void }) {
  const [remove, setRemove] = useState<SourceRecord | null>(null)
  const [confirmDirty, setConfirmDirty] = useState(false)
  const submitRegister = async (event: FormEvent<HTMLFormElement>) => { event.preventDefault(); const data = new FormData(event.currentTarget); try { await client.registerSource(String(data.get('name')), String(data.get('path'))); event.currentTarget.reset(); await reload() } catch (value) { report(value) } }
  const submitWorktree = async (event: FormEvent<HTMLFormElement>) => { event.preventDefault(); const data = new FormData(event.currentTarget); try { await client.createWorktree(String(data.get('repository')), String(data.get('ref')), String(data.get('name'))); event.currentTarget.reset(); await reload() } catch (value) { report(value) } }
  const requestRemove = (source: SourceRecord) => { setRemove(source); setConfirmDirty(false) }
  const performRemove = async () => { if (!remove) return; const dirty = Boolean(remove.inspection.identity.dirty); if (dirty && !confirmDirty) { setConfirmDirty(true); return } try { await client.removeWorktree(remove.source.name, dirty); setRemove(null); setConfirmDirty(false); await reload() } catch (value) { report(value) } }
  return <section><h1>Sources</h1><div className="source-list">{sources.map((source) => <article className="source-card" key={source.source.name}><div><h2>{source.source.name}</h2><p><span className="kind-label">{source.source.kind}</span> <span className="mono">{source.source.path}</span></p><p>{source.inspection.branch ?? source.inspection.identity.ref ?? 'detached'} @ <span className="mono">{short(source.inspection.identity.commit)}</span> · {source.inspection.identity.dirty ? `modified (${dirtyText(source)})` : 'clean'} · ↑{source.inspection.ahead ?? '?'} ↓{source.inspection.behind ?? '?'}</p></div>{source.source.kind === 'managed' && <button className="danger" onClick={() => requestRemove(source)}>Remove</button>}</article>)}</div>
    <div className="forms"><form onSubmit={submitRegister}><h2>Register unmanaged</h2><label>Name<input required name="name" /></label><label>Path<input required name="path" className="mono" /></label><button className="primary">Register source</button></form><form onSubmit={submitWorktree}><h2>Create worktree</h2><label>Repository<select required name="repository"><option value="">Choose source</option>{sources.map((source) => <option key={source.source.name}>{source.source.name}</option>)}</select></label><label>Ref<input required name="ref" className="mono" /></label><label>Name<input name="name" /></label><button className="primary">Create worktree</button></form></div>
    {remove && <div className="modal-backdrop"><div role="dialog" aria-modal="true" aria-labelledby="remove-title" className="modal"><h2 id="remove-title">Remove {remove.source.name}?</h2>{remove.inspection.identity.dirty && <p className="warning">Dirty worktree: {dirtyText(remove)}. Switchyard will not discard these changes without explicit confirmation.</p>}{confirmDirty && <p><strong>Second step:</strong> confirm removal of the dirty worktree.</p>}<div><button onClick={() => setRemove(null)}>Keep worktree</button><button className="danger" onClick={performRemove}>{remove.inspection.identity.dirty && !confirmDirty ? 'Review dirty removal' : 'Confirm removal'}</button></div></div></div>}
  </section>
}

function OperationsView({ operations, onCancel }: { operations: Operation[]; onCancel: (id: string) => void }) {
  return <section><h1>Operations</h1>{operations.length === 0 ? <p>No operations started in this GUI session.</p> : <ol className="timeline">{operations.map((operation) => <li key={operation.id}><div><span className={`status-dot status-${operation.status}`} /> <strong>{operation.kind}</strong> <span>{operation.status}</span><p className="mono">{operation.id}</p><time>{new Date(operation.startedAt).toLocaleString()}</time>{operation.result && operation.result.exitCode !== 0 && <div className="operation-error"><p>Failed command: {operation.kind}</p><p>Exit code: {operation.result.exitCode}</p><pre>{operation.result.stderr.split('\n').slice(-12).join('\n')}</pre></div>}</div>{!terminal(operation.status) && <button onClick={() => onCancel(operation.id)}>Cancel</button>}</li>)}</ol>}</section>
}

function eventText(event: OperationEvent) { return String(event.data.line ?? event.data.message ?? JSON.stringify(event.data)) }
