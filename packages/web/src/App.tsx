import { useCallback, useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from 'react'
import { ApiClient, ApiError, type AdapterRecord, type CloneChallenge, type CloneSourceRequest, type DeploymentDetail, type DeploymentSummary, type DeviceRecord, type JsonValue, type Operation, type OperationEvent, type PlannerWarning, type ProfileRecord, type ProjectInfo, type RouteHistory, type RouterBinding, type RouteState, type RunActionsResponse, type SourceRecord } from './api'
import DeploymentWorkspace, { AuthoredConnections, InstanceBindingEditor, PlannerWarnings, RoutingEditor } from './DeploymentWorkspace'
import { activeConnections, definitionSpec, type ConnectionSpec } from './connectionModel'
import DeploymentBuilder, { BlockLibrary } from './DeploymentBuilder'
import HomeView from './HomeView'
import type { DeploymentSignal, HomeDestination } from './homeModel'
import ProfilesView from './ProfilesView'
import { originLabel, trustLabel } from './profileModel'
import RunActionsView from './RunActionsView'
import './App.css'

type View = 'home' | 'deployments' | 'sources' | 'devices' | 'profiles' | 'run-actions' | 'operations' | 'builder' | 'library'
const defaultClient = new ApiClient()
const terminal = (status: Operation['status']) => ['succeeded', 'failed', 'cancelled'].includes(status)
const short = (value?: string | null) => value ? value.slice(0, 9) : 'unknown'
const stoppedDiagnostic = (detail: DeploymentDetail) => detail.resources.length === 0
  ? detail.reconciliation.diagnostics.find((diagnostic) => diagnostic.code === 'observed_resources_missing')
  : undefined
const instanceResources = (detail: DeploymentDetail, instance: string) => detail.resources.filter((resource) => resource.labels['dev.switchyard.instance'] === instance)
const serviceResources = (detail: DeploymentDetail, instance: string, service: string) => instanceResources(detail, instance).filter((resource) => resource.labels['dev.switchyard.service'] === service)
const observedPlacement = (detail: DeploymentDetail, instance: string) => [...new Set(instanceResources(detail, instance).map((resource) => resource.device))]
const observedService = (detail: DeploymentDetail, instance: string, service: string) => { const resources = serviceResources(detail, instance, service); const states = [...new Set(resources.filter((resource) => resource.kind === 'container').map((resource) => resource.state).filter((state): state is string => Boolean(state)))]; const normalized = states.join(' ').toLocaleLowerCase(); return { state: states.length ? states.join(', ') : resources.length ? 'not reported by observed resources' : 'not observed', health: normalized.includes('unhealthy') ? 'unhealthy' : normalized.includes('healthy') ? 'healthy' : resources.length ? 'not reported by observed resources' : 'not observed', placement: resources.length ? [...new Set(resources.map((resource) => resource.device))].join(', ') : 'not observed' } }
const dirtyText = (source: SourceRecord) => {
  const changes = source.inspection.changes
  return changes ? `${changes.staged} staged, ${changes.unstaged} unstaged, ${changes.untracked} untracked` : 'dirty details unavailable'
}
const routeVersion = (value: number | null) => value === null ? '—' : `v${value}`
const routeTransition = (value: JsonValue) => {
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    for (const key of ['state', 'status', 'strategy']) if (typeof value[key] === 'string') return value[key]
    if (Object.keys(value).length === 0) return 'none'
  }
  return JSON.stringify(value) ?? String(value)
}
const bindingHistory = (routes: RouteState, binding: RouterBinding) => routes.history.filter((entry) => entry.router === binding.router && entry.binding === binding.binding).slice(-5)
const rollbackDetail = (binding: RouterBinding, history: RouteHistory[]) => {
  const rollback = [...history].reverse().find((entry) => entry.activationStatus === 'rolled_back')
  if (rollback) return `rollback recorded at v${rollback.version} (timestamp ${rollback.recordedAt})`
  return binding.previousVersion === null ? 'no rollback recorded' : `previous version v${binding.previousVersion} available for rollback`
}

export default function App({ client = defaultClient }: { client?: ApiClient }) {
  const [view, setView] = useState<View>('deployments')
  const [builderDeployment, setBuilderDeployment] = useState('')
  const [project, setProject] = useState<ProjectInfo | null>(null)
  const [deployments, setDeployments] = useState<DeploymentSummary[]>([])
  const [deploymentSignals, setDeploymentSignals] = useState<DeploymentSignal[]>([])
  const [deploymentsLoading, setDeploymentsLoading] = useState(true)
  const [deploymentsUnavailable, setDeploymentsUnavailable] = useState(false)
  const [selected, setSelected] = useState('')
  const [selectedInstance, setSelectedInstance] = useState('')
  const [detail, setDetail] = useState<DeploymentDetail | null>(null)
  const [routes, setRoutes] = useState<RouteState | null>(null)
  const [sources, setSources] = useState<SourceRecord[]>([])
  const [sourcesLoading, setSourcesLoading] = useState(true)
  const [sourcesUnavailable, setSourcesUnavailable] = useState(false)
  const [devices, setDevices] = useState<DeviceRecord[]>([])
  const [devicesLoading, setDevicesLoading] = useState(true)
  const [devicesUnavailable, setDevicesUnavailable] = useState(false)
  const [adapters, setAdapters] = useState<AdapterRecord[]>([])
  const [profiles, setProfiles] = useState<ProfileRecord[]>([])
  const [profilesLoading, setProfilesLoading] = useState(true)
  const [profilesUnavailable, setProfilesUnavailable] = useState(false)
  const [profileSourceErrors, setProfileSourceErrors] = useState<Array<{ source: string; message: string }>>([])
  const [runActions, setRunActions] = useState<RunActionsResponse | null>(null)
  const [operations, setOperations] = useState<Operation[]>([])
  const [operationsLoading, setOperationsLoading] = useState(true)
  const [operationsUnavailable, setOperationsUnavailable] = useState(false)
  const [instanceOperations, setInstanceOperations] = useState<Operation[]>([])
  const [events, setEvents] = useState<OperationEvent[]>([])
  const [drawerOpen, setDrawerOpen] = useState(true)
  const [deploymentFilter, setDeploymentFilter] = useState('')
  const [textFilter, setTextFilter] = useState('')
  const [notice, setNotice] = useState('Ready')
  const [error, setError] = useState('')
  const subscriptions = useRef<Map<string, { close(): void }>>(new Map())

  const report = useCallback((value: unknown) => setError(value instanceof ApiError ? `${value.code}: ${value.message}` : String(value)), [])
  const loadDeploymentSignal = useCallback(async (summary: DeploymentSummary): Promise<DeploymentSignal> => {
    try {
      const found = await client.deployment(summary.name); const definition = await client.definition(summary.name); const validation = await client.validateDeployment(summary.name, definition.yaml); const authored = definitionSpec(validation.preview); const applied: ConnectionSpec | null = found.snapshot?.spec ?? null; const spec = authored ?? applied
      return { summary, detail: found, spec, warnings: validation.warnings ?? [], error: authored ? null : applied ? 'Validated definition preview omitted the authored spec; checklist and connection signals fall back to the applied snapshot.' : 'Deployment spec is unavailable from the validated definition preview and applied snapshot.' }
    } catch (value) { return { summary, detail: null, spec: null, error: value instanceof ApiError ? `${value.code}: ${value.message}` : String(value) } }
  }, [client])
  const loadDeployments = useCallback(async () => {
    setDeploymentsLoading(true); setDeploymentsUnavailable(false)
    try {
      const response = await client.deployments()
      setDeployments(response.deployments)
      setSelected((current) => current || response.deployments[0]?.name || '')
      if (response.deployments.length === 0) setView((current) => current === 'deployments' ? 'home' : current)
      setDeploymentSignals(await Promise.all(response.deployments.map(loadDeploymentSignal)))
    } catch (value) { setDeploymentsUnavailable(true); report(value) } finally { setDeploymentsLoading(false) }
  }, [client, loadDeploymentSignal, report])
  const loadSources = useCallback(async () => { setSourcesLoading(true); setSourcesUnavailable(false); try { setSources(await client.sources()) } catch (value) { setSourcesUnavailable(true); report(value) } finally { setSourcesLoading(false) } }, [client, report])
  const loadDevices = useCallback(async () => { setDevicesLoading(true); setDevicesUnavailable(false); try { setDevices(await client.devices()) } catch (value) { setDevicesUnavailable(true); report(value) } finally { setDevicesLoading(false) } }, [client, report])
  const loadProfiles = useCallback(async () => { setProfilesLoading(true); setProfilesUnavailable(false); try { const response = await client.profiles(); setProfiles(response.profiles); setProfileSourceErrors(response.sourceErrors) } catch (value) { setProfilesUnavailable(true); report(value) } finally { setProfilesLoading(false) } }, [client, report])
  const loadRunActions = async () => { try { setRunActions(await client.runActions()) } catch (value) { report(value) } }
  const loadOperations = useCallback(async () => {
    setOperationsLoading(true); setOperationsUnavailable(false)
    try {
      const response = await client.operations()
      setOperations((current) => response.operations.map((durable) => current.find((operation) => operation.id === durable.id && operation.result) ?? durable))
    } catch (value) { setOperationsUnavailable(true); report(value) } finally { setOperationsLoading(false) }
  }, [client, report])
  const loadSelected = useCallback(async () => { if (!selected) return; const [nextDetail, nextRoutes] = await Promise.all([client.deployment(selected), client.routes(selected)]); setDetail(nextDetail); setRoutes(nextRoutes) }, [client, selected])

  useEffect(() => { void client.project().then(setProject).catch(report); void loadDeployments(); void loadSources(); void loadDevices(); void loadProfiles(); void loadOperations(); void client.adapters().then(setAdapters).catch(report) }, [client, loadDeployments, loadDevices, loadOperations, loadProfiles, loadSources, report])
  useEffect(() => {
    setSelectedInstance('')
    if (!selected) { setDetail(null); setRoutes(null); return }
    void loadSelected().catch(report)
  }, [loadSelected, report, selected])
  useEffect(() => {
    setInstanceOperations([])
    if (!selected || !selectedInstance) return
    let active = true
    void client.operations({ deployment: selected, instance: selectedInstance }).then((response) => { if (active) setInstanceOperations(response.operations) }).catch((value) => { if (active) setError(value instanceof ApiError ? `${value.code}: ${value.message}` : String(value)) })
    return () => { active = false }
  }, [client, selected, selectedInstance])
  useEffect(() => () => { for (const subscription of subscriptions.current.values()) subscription.close() }, [])

  const observe = (started: Operation): Promise<Operation | null> => {
    setOperations((current) => [started, ...current.filter((item) => item.id !== started.id)])
    if (started.deployment === selected && started.instance === selectedInstance) setInstanceOperations((current) => [started, ...current.filter((item) => item.id !== started.id)])
    setNotice(`${started.kind} ${started.status}`)
    const subscription = client.subscribe(started.id, (event) => {
      setEvents((current) => [...current, event])
      if (event.kind !== 'log') setNotice(`${event.kind} transition`)
    }, () => setNotice(`event stream reconnecting for ${started.id}`))
    subscriptions.current.set(started.id, subscription)
    return client.pollOperation(started.id).then((finished) => {
      setOperations((current) => current.map((item) => item.id === finished.id ? finished : item))
      setInstanceOperations((current) => current.map((item) => item.id === finished.id ? finished : item))
      setNotice(`${finished.kind} ${finished.status}`)
      subscription.close()
      subscriptions.current.delete(finished.id)
      void Promise.all([loadDeployments(), loadSelected()]).catch(report)
      return finished
    }).catch((value) => { subscription.close(); subscriptions.current.delete(started.id); report(value); return null })
  }
  const runCommand = async (kind: 'validate' | 'plan' | 'status' | 'logs' | 'open' | 'apply' | 'down' | 'cleanup', target?: string) => {
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
    try { observe(await client.command(kind, bundle, { ...(kind === 'cleanup' ? { confirmed: true } : {}), ...(kind === 'logs' && target ? { target } : {}), ...(kind === 'open' && target ? { instance: target } : {}) })); setView('operations') } catch (value) { report(value) }
  }
  const navigateFromHome = (destination: HomeDestination) => { if (destination === 'builder') setBuilderDeployment(''); setView(destination); if (destination === 'operations') void loadOperations(); if (destination === 'profiles') void loadProfiles() }
  const navKeys = (event: KeyboardEvent<HTMLElement>) => {
    if (!['ArrowLeft', 'ArrowRight'].includes(event.key)) return
    const views: View[] = ['home', 'deployments', 'sources', 'devices', 'profiles', 'run-actions', 'operations', 'library']
    const offset = event.key === 'ArrowRight' ? 1 : -1
    const next = views[(views.indexOf(view) + offset + views.length) % views.length]
    setView(next)
    if (next === 'operations') void loadOperations()
    if (next === 'run-actions') void loadRunActions()
    event.preventDefault()
  }
  const eventQuery = textFilter.trim().toLocaleLowerCase()
  const visibleEvents = events.filter((event) => {
    const operation = operations.find((item) => item.id === event.operationId)
    return (!deploymentFilter || operation?.deployment === deploymentFilter) && (!eventQuery || [eventText(event), operation?.deployment, operation?.kind, operation?.id].some((value) => value?.toLocaleLowerCase().includes(eventQuery)))
  })

  return <div className="app-shell">
    <aside className="rail" aria-label="Deployment rail">
      <div className="brand">SWITCHYARD <span>LOCAL</span><small title={project?.root}>{project?.name ?? 'Loading project…'}</small></div>
      <nav aria-label="Main views" onKeyDown={navKeys}>
        {(['home', 'deployments', 'sources', 'devices', 'profiles', 'run-actions', 'operations', 'library'] as View[]).map((item) => <button key={item} aria-current={view === item ? 'page' : undefined} onClick={() => { setView(item); if (item === 'operations') void loadOperations(); if (item === 'profiles') void loadProfiles(); if (item === 'run-actions') void loadRunActions() }}>{item === 'library' ? 'block library' : item === 'run-actions' ? 'run actions' : item}</button>)}
      </nav>
      <h2>Deployments</h2>
      <div className="deployment-list">
        {deployments.length === 0 && <p className="muted">No deployments applied</p>}
        {deployments.map((deployment) => {
          const stopped = selected === deployment.name && detail?.deployment === deployment.name && stoppedDiagnostic(detail)
          const status = stopped ? 'stopped' : deployment.lastOperation?.status ?? 'unknown'
          return <button className="deployment-button" aria-pressed={selected === deployment.name} key={deployment.name} onClick={() => { setSelected(deployment.name); setView('deployments') }}>
            <span className={`status-dot status-${status}`} aria-hidden="true" />
            <span><strong>{deployment.name}</strong><small>{stopped ? 'stopped / cleaned up' : status}</small></span>
          </button>
        })}
      </div>
      <button className="new-deployment" onClick={() => { setBuilderDeployment(''); setView('builder') }}>+ New deployment</button>
    </aside>
    <main className="canvas" id="main-content">
      {error && <div className="error" role="alert"><span>{error}</span><button aria-label="Dismiss error" onClick={() => setError('')}>×</button></div>}
      {view === 'home' && <HomeView project={project} sources={sources} profiles={profiles} profileSourceErrors={profileSourceErrors} deployments={deploymentSignals} devices={devices} operations={operations} loading={{ sources: sourcesLoading, profiles: profilesLoading, deployments: deploymentsLoading, devices: devicesLoading, operations: operationsLoading }} unavailable={{ sources: sourcesUnavailable, profiles: profilesUnavailable, deployments: deploymentsUnavailable, devices: devicesUnavailable, operations: operationsUnavailable }} navigate={navigateFromHome} />}
      {view === 'deployments' && <DeploymentView client={client} detail={detail} routes={routes} warnings={deploymentSignals.find((deployment) => deployment.summary.name === selected)?.warnings ?? []} selectedInstance={selectedInstance} onSelectInstance={setSelectedInstance} onAddInstance={(deployment) => { setBuilderDeployment(deployment); setView('builder') }} onCommand={runCommand} observe={observe} refresh={async () => { await loadSelected(); await loadDeployments() }} report={report} />}
      {view === 'sources' && <SourcesView client={client} sources={sources} reload={loadSources} observe={observe} report={report} />}
      {view === 'devices' && <DevicesView client={client} devices={devices} loading={devicesLoading} reload={loadDevices} report={report} />}
      {view === 'profiles' && <ProfilesView client={client} profiles={profiles} sourceErrors={profileSourceErrors} sources={sources} reload={loadProfiles} report={report} />}
      {view === 'run-actions' && <RunActionsView client={client} response={runActions} deployments={deployments} reload={loadRunActions} startOperation={(operation) => { void observe(operation); setView('operations') }} report={report} />}
      {view === 'operations' && <OperationsView operations={operations} onCancel={async (id) => { if (!window.confirm('Cancel this running operation?')) return; try { const cancelled = await client.cancel(id); setOperations((current) => current.map((item) => item.id === id ? cancelled : item)) } catch (value) { report(value) } }} />}
      {view === 'builder' && <DeploymentBuilder client={client} projectRoot={project?.root} sources={sources} profiles={profiles} devices={devices} deployment={builderDeployment || undefined} close={() => setView('deployments')} onOperation={observe} report={report} saved={async (name) => { await loadDeployments(); setSelected(name); setView('deployments'); setNotice(builderDeployment ? `Instance appended to ${name}; use Up when ready` : `Deployment ${name} saved; use Up when ready`); setBuilderDeployment('') }} />}
      {view === 'library' && <BlockLibrary adapters={adapters} />}
    </main>
    <aside className="inspector" aria-label="Inspector">
      <h2>Inspector</h2>
      {detail ? <DeploymentInspector client={client} detail={detail} routes={routes} operations={instanceOperations} profiles={profiles} selectedInstance={selectedInstance} onOperation={observe} refresh={async () => { await loadSelected(); await loadDeployments() }} report={report} /> : <p className="muted">Select a deployment</p>}
    </aside>
    <section className={`event-drawer ${drawerOpen ? 'open' : ''}`} aria-label="Events and logs">
      <header><button aria-expanded={drawerOpen} onClick={() => setDrawerOpen((value) => !value)}>Events & logs {drawerOpen ? '▾' : '▴'}</button><label>Deployment <select value={deploymentFilter} onChange={(event) => setDeploymentFilter(event.target.value)}><option value="">All</option>{deployments.map((deployment) => <option key={deployment.name}>{deployment.name}</option>)}</select></label><label>Filter events and logs <input value={textFilter} onChange={(event) => setTextFilter(event.target.value)} /></label><button onClick={() => void navigator.clipboard?.writeText(visibleEvents.map(eventText).join('\n'))}>Copy plain text</button></header>
      {drawerOpen && <div className="log-lines" role="log">{visibleEvents.length ? visibleEvents.map((event) => <div key={`${event.operationId}-${event.id}`}><time>{new Date(event.timestamp).toLocaleTimeString()}</time> <b>{event.kind}</b> {eventText(event)}</div>) : <p>No events yet.</p>}</div>}
    </section>
    <div className="sr-only" aria-live="polite" aria-atomic="true">{notice}</div>
  </div>
}

function DeploymentView({ client, detail, routes, warnings, selectedInstance, onSelectInstance, onAddInstance, onCommand, observe, refresh, report }: { client: ApiClient; detail: DeploymentDetail | null; routes: RouteState | null; warnings: PlannerWarning[]; selectedInstance: string; onSelectInstance: (instance: string) => void; onAddInstance: (deployment: string) => void; onCommand: (kind: 'validate' | 'plan' | 'status' | 'logs' | 'open' | 'apply' | 'down' | 'cleanup', target?: string) => void; observe: (operation: Operation) => Promise<Operation | null>; refresh: () => Promise<void>; report: (error: unknown) => void }) {
  if (!detail) return <section><h1>Deployments</h1><p>No applied deployment selected.</p></section>
  const instances = detail.snapshot?.spec?.instances ?? Object.keys(detail.sourceIdentities).map((name) => ({ name, device: undefined }))
  const stopped = stoppedDiagnostic(detail)
  return <section><div className="title-row"><div><p className="eyebrow">Deployment</p><h1>{detail.deployment}</h1></div><span className={`state-label ${stopped ? 'state-stopped' : ''}`}>● {stopped ? 'Stopped / cleaned up' : detail.reconciliation.diagnostics.length ? 'Needs attention' : 'Reconciled'}</span></div>
    <div className="command-bar" aria-label="Deployment commands"><button className="primary" onClick={() => onAddInstance(detail.deployment)}>Add instance</button><button onClick={() => onCommand('validate')}>Validate</button><button onClick={() => onCommand('plan')}>Plan</button><button onClick={() => onCommand('status')}>Status</button><button disabled={Boolean(stopped)} title={stopped ? 'Start the deployment to view runtime logs' : undefined} onClick={() => onCommand('logs')}>Logs</button><button className="primary" onClick={() => onCommand('apply')}>Up</button><button className="danger" disabled={Boolean(stopped)} onClick={() => onCommand('down')}>Down</button><button className="danger" disabled={Boolean(stopped)} onClick={() => onCommand('cleanup')}>Cleanup</button></div>
    <PlannerWarnings warnings={warnings} />
    {stopped && <section className="stopped-callout" role="status"><div><h2>Deployment is stopped or cleaned up</h2><p>There is no running endpoint or live route topology for this deployment.</p><p><strong>Reconciliation:</strong> {stopped.message}</p></div><button className="primary" onClick={() => onCommand('apply')}>Run Up</button></section>}
    <h2>Instances</h2><div className="instance-grid">{instances.map((instance) => { const identity = detail.sourceIdentities[instance.name]; const resources = instanceResources(detail, instance.name); const resource = resources[0]; const observedDevices = observedPlacement(detail, instance.name); return <article className="instance-card" data-selected={selectedInstance === instance.name || undefined} key={instance.name}><header><h3>{instance.name}</h3><span>{stopped ? 'not running' : resource?.state ?? 'state unknown'}</span><button aria-label={`Inspect ${instance.name}`} onClick={() => onSelectInstance(instance.name)}>Inspect</button><button disabled={Boolean(stopped)} aria-label={`Logs for ${instance.name}`} onClick={() => onCommand('logs', instance.name)}>Logs</button>{detail.snapshot?.spec?.managedProfiles?.[instance.name] && <button disabled={Boolean(stopped)} aria-label={`Open ${instance.name} in a managed Chromium profile`} onClick={() => onCommand('open', instance.name)}>Managed profile</button>}</header><dl><dt>Authored placement</dt><dd className="mono">{instance.device ?? 'local'}</dd><dt>Observed placement</dt><dd className="mono">{observedDevices.length ? observedDevices.join(', ') : 'not observed'}</dd>{identity && <><dt>Path</dt><dd className="mono">{identity.path}</dd><dt>Ref</dt><dd className="mono">{identity.ref ?? 'detached'}</dd><dt>Commit</dt><dd className="mono">{short(identity.commit)} {identity.dirty ? <span className="dirty">● modified</span> : 'clean'}</dd></>}</dl>{!identity && <p>Source identity unavailable</p>}</article> })}</div>
    {stopped ? <AuthoredConnections client={client} deployment={detail.deployment} onSaved={refresh} report={report} /> : <DeploymentWorkspace detail={detail} selectedInstance={selectedInstance} onSelectInstance={onSelectInstance} />}
    <h2>Active routes</h2>{stopped ? <p className="muted">No routes are active while the deployment is stopped.</p> : routes?.bindings.length ? <><table><thead><tr><th>Instance</th><th>Router</th><th>Desired</th><th>Observed</th><th>Previous</th><th>Transition</th><th>Status</th><th>Rollback</th></tr></thead><tbody>{routes.bindings.map((route) => { const history = bindingHistory(routes, route); return <tr key={`${route.router}-${route.binding}`}><td className="mono">{route.binding}</td><td className="mono">{route.router}</td><td className="mono">{routeVersion(route.desiredVersion)}</td><td className="mono">{routeVersion(route.observedVersion)}</td><td className="mono">{routeVersion(route.previousVersion)}</td><td>{routeTransition(route.transition)}</td><td>{route.status}{route.lastErrorCode ? ` · ${route.lastErrorCode}` : ''}</td><td>{rollbackDetail(route, history)}</td></tr> })}</tbody></table><h3>Route activation and rollback history</h3>{routes.bindings.some((route) => bindingHistory(routes, route).length) ? <table><thead><tr><th>Instance</th><th>Router</th><th>Version</th><th>Activation</th><th>Recorded timestamp</th><th>Operation</th></tr></thead><tbody>{routes.bindings.flatMap((route) => bindingHistory(routes, route).map((entry) => <tr key={entry.sequence}><td className="mono">{route.binding}</td><td className="mono">{route.router}</td><td className="mono">v{entry.version}</td><td>{entry.activationStatus}</td><td className="mono">{entry.recordedAt}</td><td className="mono">{entry.operationId ?? '—'}</td></tr>))}</tbody></table> : <p className="muted">No route activation or rollback history recorded.</p>}</> : <p className="muted">No active route versions recorded.</p>}
    <RoutingEditor client={client} deployment={detail.deployment} onSaved={refresh} onOperation={observe} report={report} />
  </section>
}

function DeploymentInspector({ client, detail, routes, operations, profiles, selectedInstance, onOperation, refresh, report }: { client: ApiClient; detail: DeploymentDetail; routes: RouteState | null; operations: Operation[]; profiles: ProfileRecord[]; selectedInstance: string; onOperation: (operation: Operation) => Promise<Operation | null>; refresh: () => Promise<void>; report: (error: unknown) => void }) {
  const stopped = stoppedDiagnostic(detail); const spec = detail.snapshot?.spec
  const instance = spec?.instances?.find((item) => item.name === selectedInstance)
  if (selectedInstance && instance) {
    const identity = detail.sourceIdentities[selectedInstance]; const block = instance.block ? spec?.blocks?.[instance.block] : undefined; const services = Object.keys(block?.services ?? {}); const observedDevices = observedPlacement(detail, selectedInstance); const connections = activeConnections(spec ?? {}, selectedInstance); const recent = operations.slice(0, 5); const profile = instance.block ? profiles.find((record) => record.name === instance.block && record.deployment === detail.deployment) : undefined
    return <section className="instance-inspector" aria-label={`Selected instance ${selectedInstance}`}><p className="eyebrow">Instance</p><h3>{selectedInstance}</h3><dl><dt>Startup profile</dt><dd className="mono">{instance.block ?? 'not recorded'}{profile ? ` · ${originLabel(profile)} · ${trustLabel(profile.trust)}` : instance.block ? ' · not listed in the profile library' : ''}</dd><dt>Authored placement</dt><dd className="mono">{instance.device ?? 'local'}</dd><dt>Observed placement</dt><dd className="mono">{observedDevices.length ? observedDevices.join(', ') : 'not observed'}</dd><dt>Source</dt><dd className="mono">{instance.source ?? 'not recorded'}</dd>{identity && <><dt>Path</dt><dd className="mono">{identity.path}</dd><dt>Ref</dt><dd className="mono">{identity.ref ?? 'detached'}</dd><dt>Commit</dt><dd className="mono">{short(identity.commit)}</dd></>}</dl>
      <h3>Expanded services</h3><p className="help">Service names come from the applied snapshot. Observed state, health, and placement use the persisted instance and service ownership labels; resources recorded before those labels were retained remain honestly unavailable until observed again.</p>{services.length ? <ul className="inspector-services">{services.map((service) => { const observed = observedService(detail, selectedInstance, service); return <li key={service}><strong>{service}</strong><dl><dt>State</dt><dd>{observed.state}</dd><dt>Health</dt><dd>{observed.health}</dd><dt>Resource placement</dt><dd>{observed.placement}</dd></dl></li> })}</ul> : <p className="muted">No services are declared by this expanded block.</p>}
      <h3>Group membership</h3>{connections.length ? <ul>{connections.map((connection) => <li key={`${connection.group}-${connection.member}`}><strong>{connection.disabled ? 'Disabled' : 'Active'}</strong> <span className="mono">{connection.group} / {connection.member}</span></li>)}</ul> : <p className="muted">This instance is not in a group.</p>}
      {!stopped && <InstanceBindingEditor client={client} detail={detail} routes={routes} instance={selectedInstance} onOperation={onOperation} refresh={refresh} report={report} />}
      <h3>Recent operations</h3><p className="help">Only operations durably attributed to instance <span className="mono">{selectedInstance}</span> are shown. Deployment-wide operations and legacy records whose instance is null are not blended into this list.</p>{recent.length ? <ol className="inspector-operations">{recent.map((operation) => <li key={operation.id}><strong>{operation.kind}</strong> — {operation.status}<br /><span className="mono">{operation.id}</span></li>)}</ol> : <p className="muted">No instance-scoped operations recorded for {selectedInstance}.</p>}
    </section>
  }
  return <><p className="eyebrow">Deployment</p><h3>{detail.deployment}</h3><p className="muted">Select Inspect on an instance card or select an instance in the runtime patch bay for per-instance details.</p><dl><dt>State</dt><dd>{stopped ? 'Stopped / cleaned up' : 'Active'}</dd><dt>Definition</dt><dd className="mono">{short(detail.definitionHash)}</dd><dt>Resources</dt><dd className="mono">{short(detail.resourceHash)}</dd><dt>Drift</dt><dd>{detail.reconciliation.diagnostics.length ? `${detail.reconciliation.diagnostics.length} warnings` : 'Reconciled'}</dd></dl>
    {detail.reconciliation.diagnostics.length > 0 && <><h3>Reconciliation</h3><ul className="diagnostic-list">{detail.reconciliation.diagnostics.map((diagnostic) => <li key={`${diagnostic.code}-${diagnostic.path}`}><strong>{diagnostic.code}</strong><br />{diagnostic.message}</li>)}</ul></>}
    <h3>Runtime domains</h3>{stopped ? <p className="muted">Unavailable while stopped</p> : detail.customDomains.length ? <ul>{detail.customDomains.map((domain) => { const links = detail.customDomainLinks?.filter((link) => link.domain === domain) ?? []; return <li className="mono" key={domain}>{links.length ? links.map((link) => <a href={link.url} key={link.url} target="_blank" rel="noreferrer" aria-label={`Open ${domain} in the default browser`}>{domain}</a>) : domain}</li> })}</ul> : <p className="muted">None</p>}
    <h3>{stopped ? 'Saved memberships' : 'Memberships'}</h3><dl>{Object.entries(detail.memberships).map(([instanceName, group]) => <div key={instanceName}><dt className="mono">{instanceName}</dt><dd>{group}</dd></div>)}</dl></>
}

function SourcesView({ client, sources, reload, observe, report }: { client: ApiClient; sources: SourceRecord[]; reload: () => Promise<void>; observe: (operation: Operation) => void; report: (error: unknown) => void }) {
  const [remove, setRemove] = useState<SourceRecord | null>(null)
  const [confirmDirty, setConfirmDirty] = useState(false)
  const [clone, setClone] = useState<CloneSourceRequest | null>(null)
  const [challenge, setChallenge] = useState<CloneChallenge | null>(null)
  const finishClone = async (started: Operation) => { observe(started); const finished = await client.pollOperation(started.id); if (finished.status === 'succeeded') { setClone(null); setChallenge(null); await reload(); return } const next = finished.error?.context as CloneChallenge | undefined; if (next?.kind === 'credentials' || next?.kind === 'host_key') setChallenge(next); else throw new ApiError(400, finished.error ?? { code: 'clone_failed', message: 'Git clone failed' }) }
  const submitClone = async (event: FormEvent<HTMLFormElement>) => { event.preventDefault(); const form = event.currentTarget; const data = new FormData(form); const request: CloneSourceRequest = { name: String(data.get('name')), repository: String(data.get('repository')), ...(String(data.get('ref')) ? { ref: String(data.get('ref')) } : {}), ...(String(data.get('sshIdentityFile')) ? { sshIdentityFile: String(data.get('sshIdentityFile')) } : {}) }; setClone(request); setChallenge(null); try { const pending = client.cloneSource(request); form.reset(); await finishClone(await pending) } catch (value) { report(value) } }
  const submitCredentials = async (event: FormEvent<HTMLFormElement>) => { event.preventDefault(); if (!clone) return; const form = event.currentTarget; const data = new FormData(form); try { const pending = client.cloneSource({ ...clone, credentials: { username: String(data.get('username')), password: String(data.get('password')) } }); form.reset(); setChallenge(null); await finishClone(await pending) } catch (value) { report(value) } }
  const approveHostKey = async () => { if (!clone || challenge?.kind !== 'host_key' || !challenge.host || !challenge.fingerprint) return; try { setChallenge(null); await finishClone(await client.cloneSource({ ...clone, approvedHostKey: { host: challenge.host, fingerprint: challenge.fingerprint } })) } catch (value) { report(value) } }
  const submitRegister = async (event: FormEvent<HTMLFormElement>) => { event.preventDefault(); const data = new FormData(event.currentTarget); try { await client.registerSource(String(data.get('name')), String(data.get('path'))); event.currentTarget.reset(); await reload() } catch (value) { report(value) } }
  const submitWorktree = async (event: FormEvent<HTMLFormElement>) => { event.preventDefault(); const data = new FormData(event.currentTarget); try { await client.createWorktree(String(data.get('repository')), String(data.get('ref')), String(data.get('name'))); event.currentTarget.reset(); await reload() } catch (value) { report(value) } }
  const requestRemove = (source: SourceRecord) => { setRemove(source); setConfirmDirty(false) }
  const performRemove = async () => { if (!remove) return; const managed = remove.source.kind === 'managed'; const dirty = managed && Boolean(remove.inspection.identity.dirty); if (dirty && !confirmDirty) { setConfirmDirty(true); return } try { if (managed) await client.removeWorktree(remove.source.name, dirty); else await client.deregisterSource(remove.source.name); setRemove(null); setConfirmDirty(false); await reload() } catch (value) { report(value) } }
  return <section><h1>Sources</h1><div className="source-list">{sources.map((source) => <article className="source-card" key={source.source.name}><div><h2>{source.source.name}</h2><p><span className="kind-label">{source.source.kind}</span> <span className="mono">{source.source.path}</span></p><p>{source.inspection.branch ?? source.inspection.identity.ref ?? 'detached'} @ <span className="mono">{short(source.inspection.identity.commit)}</span> · {source.inspection.identity.dirty ? `modified (${dirtyText(source)})` : 'clean'} · ↑{source.inspection.ahead ?? '?'} ↓{source.inspection.behind ?? '?'}</p></div><button className="danger" onClick={() => requestRemove(source)}>Remove</button></article>)}</div>
    <div className="forms"><form onSubmit={submitClone}><h2>Clone Git repository</h2><p className="muted">Switchyard tries your credential helper or SSH agent first. If needed, credentials are requested for one attempt and never saved. Use an <span className="mono">https://</span> or SSH URL — Git sends credentials unencrypted over plain <span className="mono">http://</span>, so that is refused for remote hosts.</p><label>Name<input required name="name" /></label><label>Repository URL<input required name="repository" className="mono" /></label><label>Ref <span className="muted">(optional)</span><input name="ref" className="mono" /></label><label>SSH identity file <span className="muted">(optional)</span><input name="sshIdentityFile" className="mono" /></label><button className="primary">Clone repository</button></form><form onSubmit={submitRegister}><h2>Register unmanaged</h2><label>Name<input required name="name" /></label><label>Path<input required name="path" className="mono" /></label><button className="primary">Register source</button></form><form onSubmit={submitWorktree}><h2>Create worktree</h2><label>Repository<select required name="repository"><option value="">Choose source</option>{sources.map((source) => <option key={source.source.name}>{source.source.name}</option>)}</select></label><label>Ref<input required name="ref" className="mono" /></label><label>Name<input name="name" /></label><button className="primary">Create worktree</button></form></div>
    {challenge?.kind === 'credentials' && <div className="modal-backdrop"><form role="dialog" aria-modal="true" aria-labelledby="clone-credentials-title" className="modal" onSubmit={submitCredentials}><h2 id="clone-credentials-title">Git credentials required</h2><p>These credentials are used for one clone attempt, pass through memory only, and are not saved or shown again. They are sent to <span className="mono">{clone?.repository}</span> over an encrypted connection; a plain <span className="mono">http://</span> URL to a remote host is refused.</p><label>Username<input required name="username" autoComplete="username" /></label><label>Password or token<input required name="password" type="password" autoComplete="current-password" /></label><div><button type="button" onClick={() => { setChallenge(null); setClone(null) }}>Cancel clone</button><button className="primary">Retry clone</button></div></form></div>}
    {challenge?.kind === 'host_key' && <div className="modal-backdrop"><div role="dialog" aria-modal="true" aria-labelledby="clone-host-key-title" className="modal"><h2 id="clone-host-key-title">Approve SSH host key?</h2><p>Verify this fingerprint for <strong>{challenge.host}</strong> through a trusted channel before approving.</p><pre>{challenge.fingerprint}</pre><div><button onClick={() => { setChallenge(null); setClone(null) }}>Cancel clone</button><button className="primary" onClick={() => void approveHostKey()}>Approve this fingerprint</button></div></div></div>}
    {remove && <div className="modal-backdrop"><div role="dialog" aria-modal="true" aria-labelledby="remove-title" className="modal"><h2 id="remove-title">Remove {remove.source.name}?</h2><p>{remove.source.kind === 'managed' ? 'This deletes the managed worktree directory from disk.' : 'This forgets only the registration. Files on disk are untouched.'}</p>{remove.source.kind === 'managed' && remove.inspection.identity.dirty && <p className="warning">Dirty worktree: {dirtyText(remove)}. Switchyard will not discard these changes without explicit confirmation.</p>}{confirmDirty && <p><strong>Second step:</strong> confirm removal of the dirty worktree.</p>}<div><button onClick={() => setRemove(null)}>{remove.source.kind === 'managed' ? 'Keep worktree' : 'Keep registration'}</button><button className="danger" onClick={performRemove}>{remove.source.kind === 'managed' && remove.inspection.identity.dirty && !confirmDirty ? 'Review dirty removal' : 'Confirm removal'}</button></div></div></div>}
  </section>
}

function DevicesView({ client, devices, loading, reload, report }: { client: ApiClient; devices: DeviceRecord[]; loading: boolean; reload: () => Promise<void>; report: (error: unknown) => void }) {
  const [errors, setErrors] = useState<Record<string, string>>({})
  const [checking, setChecking] = useState('')
  const [remove, setRemove] = useState<DeviceRecord | null>(null)
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const form = event.currentTarget; const data = new FormData(form)
    const name = String(data.get('name')).trim(); const user = String(data.get('user')).trim(); const host = String(data.get('host')).trim(); const port = Number(data.get('port')); const identityFile = String(data.get('identityFile')).trim()
    const next: Record<string, string> = {}
    if (!name) next.name = 'Name is required.'
    if (!user || /\s/.test(user)) next.user = 'User is required and cannot contain whitespace.'
    if (!host || /\s/.test(host)) next.host = 'Host is required and cannot contain whitespace.'
    if (!Number.isInteger(port) || port < 1 || port > 65535) next.port = 'Port must be between 1 and 65535.'
    setErrors(next); if (Object.keys(next).length) return
    try { await client.addDevice({ name, user, host, port, ...(identityFile ? { identityFile } : {}) }); form.reset(); setErrors({}); await reload() } catch (value) { report(value) }
  }
  const check = async (name: string) => { setChecking(name); try { await client.checkDevice(name); await reload() } catch (value) { report(value) } finally { setChecking('') } }
  const confirmRemove = async () => { if (!remove || remove.placedInstances.length) return; try { await client.removeDevice(remove.name); setRemove(null); await reload() } catch (value) { report(value) } }
  return <section><h1>Devices</h1><p className="muted">Reachability reports the SSH path. Eligibility separately reports whether Switchyard can execute containers there.</p>
    {loading ? <p role="status">Loading devices…</p> : <table className="devices-table"><thead><tr><th>Name</th><th>Target</th><th>Reachability</th><th>Eligibility</th><th>Last checked</th><th>Actions</th></tr></thead><tbody>{devices.map((device) => <tr key={device.name}><th>{device.name}</th><td className="mono">{device.kind === 'local' ? 'this device' : `${device.user}@${device.host}:${device.port}`}</td><td><span className={`device-status device-status-${device.reachability}`}>{device.reachability}</span></td><td><span className={`device-status device-status-${device.eligibility}`}>{device.eligibility}</span><small className="device-reason">{device.eligibilityReason}</small></td><td>{device.lastCheckedAt ? new Date(device.lastCheckedAt).toLocaleString() : device.kind === 'local' ? 'Implicit' : 'Never'}</td><td className="device-actions">{device.kind === 'ssh' && <><button disabled={checking === device.name} onClick={() => void check(device.name)}>{checking === device.name ? 'Checking…' : 'Check eligibility'}</button><button className="danger" onClick={() => setRemove(device)}>Remove</button></>}</td></tr>)}</tbody></table>}
    <form className="device-form" noValidate onSubmit={submit}><h2>Add device</h2><label>Name<input name="name" aria-describedby="device-name-error" /></label>{errors.name && <p className="field-error" id="device-name-error">{errors.name}</p>}<label>User<input name="user" /></label>{errors.user && <p className="field-error">{errors.user}</p>}<label>Host<input name="host" /></label>{errors.host && <p className="field-error">{errors.host}</p>}<label>Port<input name="port" type="number" defaultValue="22" min="1" max="65535" /></label>{errors.port && <p className="field-error">{errors.port}</p>}<label>Identity file path <span className="muted">(optional)</span><input name="identityFile" className="mono" /></label><button className="primary">Add device</button></form>
    {remove && <div className="modal-backdrop"><div role="dialog" aria-modal="true" aria-labelledby="remove-device-title" className="modal"><h2 id="remove-device-title">Remove {remove.name}?</h2><p>This removes only the registry entry. SSH keys and configuration are unchanged.</p>{remove.placedInstances.length > 0 && <div className="warning"><p><strong>Removal blocked:</strong> move or remove these authored instances first.</p><ul>{remove.placedInstances.map((placement) => <li key={`${placement.deployment}-${placement.instance}`}><span className="mono">{placement.deployment} / {placement.instance}</span></li>)}</ul></div>}<div><button onClick={() => setRemove(null)}>Keep device</button><button className="danger" disabled={remove.placedInstances.length > 0} onClick={() => void confirmRemove()}>Confirm removal</button></div></div></div>}
  </section>
}

function OperationsView({ operations, onCancel }: { operations: Operation[]; onCancel: (id: string) => void }) {
  const [query, setQuery] = useState('')
  const needle = query.trim().toLocaleLowerCase()
  const visible = needle ? operations.filter((operation) => [operation.deployment, operation.kind, operation.id, operation.status, operation.result?.stdout, operation.result?.stderr].some((value) => value?.toLocaleLowerCase().includes(needle))) : operations
  return <section><h1>Operations</h1><label className="timeline-filter">Filter operations <input value={query} onChange={(event) => setQuery(event.target.value)} /></label><p className="muted">Searches the loaded page of durable operations by deployment, kind, id, status, and captured output.</p>{operations.length === 0 ? <p>No durable operations recorded.</p> : visible.length === 0 ? <p>No operations match this filter.</p> : <ol className="timeline">{visible.map((operation) =><li key={operation.id}><div><span className={`status-dot status-${operation.status}`} /> <strong>{operation.kind}</strong> <span>{operation.status}</span>{operation.destructive && <span className="destructive-marker">Destructive</span>}<p className="mono">{operation.id}</p><time>{new Date(operation.startedAt).toLocaleString()}</time>{operation.result && operation.result.exitCode !== 0 && <div className="operation-error"><p>Failed command: {operation.kind}</p><p>Exit code: {operation.result.exitCode}</p><pre>{operation.result.stderr.split('\n').slice(-12).join('\n')}</pre></div>}</div>{!terminal(operation.status) && <button onClick={() => onCancel(operation.id)}>Cancel</button>}</li>)}</ol>}</section>
}

function eventText(event: OperationEvent) { return String(event.data.line ?? event.data.message ?? JSON.stringify(event.data)) }
