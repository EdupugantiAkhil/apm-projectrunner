import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { ApiClient, DeploymentDefinition, DeploymentDetail, JsonValue, Operation, PlannerWarning, RouteHistory, RouterBinding, RouteState } from './api'
import { definitionSpec, membershipByInstance, resolvedGroups, updateGroupInstancesYaml, type ConnectionSpec } from './connectionModel'

type Pending = { instance: string; oldGroup: string | null; newGroup: string; oldRoutes: Record<string, string>; newRoutes: Record<string, string>; version: number | null }
type Transition = 'close' | 'drain' | 'pin'
type SwitchReport = { instance: string; succeeded: boolean; detail: string; routes: RouteState | null }
const emptySpec: ConnectionSpec = {}
const short = (value?: string | null) => value ? value.slice(0, 9) : 'unknown'
const version = (value: number | null) => value === null ? '—' : `v${value}`
const transitionState = (value: JsonValue) => {
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    for (const key of ['state', 'status', 'strategy']) if (typeof value[key] === 'string') return value[key]
    if (Object.keys(value).length === 0) return 'none'
  }
  return JSON.stringify(value) ?? String(value)
}
const historyFor = (routes: RouteState, binding: RouterBinding) => routes.history.filter((entry) => entry.binding === binding.binding && entry.router === binding.router).slice(-5)
const rollback = (binding: RouterBinding, history: RouteHistory[]) => {
  const recorded = [...history].reverse().find((entry) => entry.activationStatus === 'rolled_back')
  if (recorded) return `rollback recorded at v${recorded.version} (timestamp ${recorded.recordedAt})`
  return binding.previousVersion === null ? 'no rollback recorded' : `previous version v${binding.previousVersion} available for rollback`
}
const operationDetail = (operation: Operation | null) => {
  if (!operation) return 'Operation status unavailable after the switch request.'
  if (operation.error) return `${operation.error.code}: ${operation.error.message}`
  if (operation.result) return `Exit code ${operation.result.exitCode}. ${operation.result.stderr.trim() || operation.result.stdout.trim() || 'No command output.'}`
  return `${operation.kind} ${operation.status}.`
}

export function PlannerWarnings({ warnings }: { warnings: PlannerWarning[] }) {
  if (!warnings.length) return null
  return <aside className="planner-warnings" aria-label="Planner warnings"><h3>Warnings</h3><ul>{warnings.map((warning) => <li key={`${warning.code}:${warning.path}:${warning.message}`}><strong>{warning.code}</strong> at <code>{warning.path}</code>: {warning.message}</li>)}</ul></aside>
}

export default function DeploymentWorkspace({ detail, selectedInstance, onSelectInstance }: { detail: DeploymentDetail; selectedInstance: string; onSelectInstance: (instance: string) => void }) {
  const [table, setTable] = useState(false)
  const spec = detail.snapshot?.spec ?? emptySpec; const groups = useMemo(() => resolvedGroups(spec), [spec]); const instances = spec.instances ?? Object.keys(detail.sourceIdentities).map((name) => ({ name }))
  return <>
    <div className="patch-toolbar"><div><h2>Observed group membership</h2><p className="muted">Ordered members from the applied snapshot. Members share one localhost.</p></div><label className="check"><input type="checkbox" checked={table} onChange={(event) => setTable(event.target.checked)} />Membership table</label></div>
    <div className={table ? 'patch-bay table-mode' : 'patch-bay'}>
      <div className="lane"><h3>Instances</h3>{instances.map((instance) => <button className="patch-node" aria-pressed={selectedInstance === instance.name} key={instance.name} onClick={() => onSelectInstance(instance.name)}><strong>{instance.name}</strong><small>{short(detail.sourceIdentities[instance.name]?.commit)}</small></button>)}</div>
      <div className="lane group-lane"><h3>Groups</h3>{Object.entries(groups).map(([name, members]) => <div className="patch-node" key={name}><strong>{name}</strong><small>{members.length} active members</small></div>)}</div>
      <table className="route-matrix"><caption>Ordered group membership</caption><thead><tr><th>Group</th><th>Priority</th><th>Member</th></tr></thead><tbody>{Object.entries(groups).flatMap(([group, members]) => members.map((member, index) => <tr key={`${group}-${member}`}><th>{group}</th><td>{index + 1}</td><td>{member}</td></tr>))}</tbody></table>
    </div>
  </>
}

export function InstanceBindingEditor({ client, detail, routes, instance, onOperation, refresh, report }: { client: ApiClient; detail: DeploymentDetail; routes: RouteState | null; instance: string; onOperation: (operation: Operation) => Promise<Operation | null>; refresh: () => Promise<void>; report: (error: unknown) => void }) {
  const [pending, setPending] = useState<Pending | null>(null); const [switchReport, setSwitchReport] = useState<SwitchReport | null>(null); const [transition, setTransition] = useState<Transition>('close'); const [timeout, setTimeoutValue] = useState(30000)
  const spec = detail.snapshot?.spec ?? {}; const groups = resolvedGroups(spec); const memberships = Object.keys(detail.memberships).length ? detail.memberships : membershipByInstance(spec)
  const routesFor = (group = memberships[instance]) => Object.fromEntries((group && groups[group] ? groups[group] : []).map((member) => [member, member])); const compatible = Object.keys(groups)
  const prepare = (newGroup: string) => { const oldRoutes = routesFor(); const newRoutes = { ...routesFor(newGroup), [instance]: instance }; setPending({ instance, oldGroup: memberships[instance] ?? null, newGroup, oldRoutes, newRoutes, version: routes?.bindings.find((route) => route.binding === instance)?.currentVersion ?? null }) }
  const apply = async () => { if (!pending) return; const movedInstance = pending.instance; try { const extra: Record<string, unknown> = { instance: movedInstance, group: pending.newGroup, transition: transition === 'drain' ? { strategy: 'drain', timeoutMs: timeout } : { strategy: transition } }; const operation = await client.command('membership', `.switchyard/generated/${detail.deployment}/resolved-deployment.yaml`, extra); setPending(null); const finished = await onOperation(operation); let latest = routes; try { latest = await client.routes(detail.deployment) } catch (error) { report(error) } await refresh().catch(report); setSwitchReport({ instance: movedInstance, succeeded: finished?.status === 'succeeded', detail: operationDetail(finished), routes: latest }) } catch (error) { setPending(null); report(error); setSwitchReport({ instance: movedInstance, succeeded: false, detail: error instanceof Error ? error.message : String(error), routes }) } }
  return <><label>Group membership<select aria-label={`Group for ${instance}`} value={memberships[instance] ?? ''} onChange={(event) => event.target.value && prepare(event.target.value)}>{!memberships[instance] && <option value="">Not in a group</option>}{compatible.map((group) => <option key={group}>{group}</option>)}</select><span className="help">Moving the instance changes the complete ordered membership of both groups.</span></label>{pending && <ChangePreview pending={pending} transition={transition} setTransition={setTransition} timeout={timeout} setTimeout={setTimeoutValue} apply={apply} cancel={() => setPending(null)} />}{switchReport && <SwitchResult report={switchReport} close={() => setSwitchReport(null)} />}</>
}

function SwitchResult({ report, close }: { report: SwitchReport; close: () => void }) {
  const matching = report.routes?.bindings.filter((binding) => binding.binding === report.instance) ?? []
  return <div className="modal-backdrop"><div className="modal change-preview" role="dialog" aria-modal="true" aria-labelledby="switch-result-title"><h2 id="switch-result-title">Membership move result</h2><p><strong>{report.succeeded ? 'Atomic membership operation succeeded.' : 'Atomic membership operation failed.'}</strong></p><p className="mono">{report.detail}</p>{matching.length === 0 ? <><p>Route status: no durable router observation is available yet.</p>{!report.succeeded && <p>Rollback information: unavailable from route status.</p>}</> : <><h3>Router observations</h3><ul>{matching.map((binding) => { const history = report.routes ? historyFor(report.routes, binding) : []; return <li key={`${binding.router}-${binding.binding}`}><span className="mono">{binding.router}</span> — desired {version(binding.desiredVersion)}; observed {version(binding.observedVersion)}; status {binding.status}; transition {transitionState(binding.transition)}; error {binding.lastErrorCode ?? 'none'}; {rollback(binding, history)}</li> })}</ul></>}<div><button className="primary" onClick={close}>Close report</button></div></div></div>
}

function ChangePreview({ pending, transition, setTransition, timeout, setTimeout, apply, cancel }: { pending: Pending; transition: Transition; setTransition: (value: Transition) => void; timeout: number; setTimeout: (value: number) => void; apply: () => void; cancel: () => void }) {
  const members = Array.from(new Set([...Object.keys(pending.oldRoutes), ...Object.keys(pending.newRoutes)]))
  return <div className="modal-backdrop"><div className="modal change-preview" role="dialog" aria-modal="true" aria-labelledby="change-title"><h2 id="change-title">Preview membership move</h2><p><strong>{pending.instance}</strong>: {pending.oldGroup ? <>{pending.oldGroup} → {pending.newGroup}. Snapshot v{pending.version ?? 'unknown'} will be superseded.</> : <>The instance will join {pending.newGroup} at the end of its ordered list.</>}</p><table><thead><tr><th>Member</th><th>Old group view</th><th>New group view</th></tr></thead><tbody>{members.map((member) => <tr key={member}><th>{member}</th><td>{pending.oldRoutes[member] ?? (pending.oldGroup ? 'none' : 'Not in a current group')}</td><td>{pending.newRoutes[member] ?? 'none'}</td></tr>)}</tbody></table><label>Existing connections<select value={transition} onChange={(event) => setTransition(event.target.value as Transition)}><option value="close">Close</option><option value="drain">Drain</option><option value="pin">Pin</option></select></label>{transition === 'drain' && <label>Drain timeout (ms)<input type="number" min="0" value={timeout} onChange={(event) => setTimeout(Number(event.target.value))} /></label>}<div><button onClick={cancel}>Cancel</button><button className="primary" onClick={apply}>Move instance</button></div></div></div>
}

export function AuthoredConnections({ client, deployment, onSaved, report }: { client: ApiClient; deployment: string; onSaved: () => Promise<void>; report: (error: unknown) => void }) {
  const [definition, setDefinition] = useState<DeploymentDefinition | null>(null); const [spec, setSpec] = useState<ConnectionSpec | null>(null); const [draft, setDraft] = useState<Record<string, string[]>>({}); const [warnings, setWarnings] = useState<PlannerWarning[]>([]); const [saving, setSaving] = useState(false); const reportRef = useRef(report); reportRef.current = report
  const install = useCallback(async (found: DeploymentDefinition) => { const validation = await client.validateDeployment(deployment, found.yaml); const authored = definitionSpec(validation.preview); if (!authored) throw new Error('Validated definition preview did not include authored connection state.'); setDefinition(found); setSpec(authored); setDraft(Object.fromEntries(Object.entries(authored.groups ?? {}).map(([name, group]) => [name, group.instances ?? []]))); setWarnings(validation.warnings ?? []) }, [client, deployment])
  useEffect(() => { let active = true; void client.definition(deployment).then(async (found) => { if (active) await install(found) }).catch((error) => reportRef.current(error)); return () => { active = false } }, [client, deployment, install])
  if (!definition || !spec) return <section className="authored-connections" aria-label="Desired connections"><h2>Loading desired connections</h2><p>Loading desired/authored state from the deployment definition…</p></section>
  const groups = spec.groups ?? {}; const changed = Object.keys(groups).filter((group) => JSON.stringify(draft[group] ?? []) !== JSON.stringify(groups[group].instances ?? []))
  const save = async () => { if (!changed.length) return; setSaving(true); try { let yaml = definition.yaml; for (const group of changed) yaml = updateGroupInstancesYaml(yaml, group, draft[group] ?? []); const saved = await client.updateDefinitionValidated(deployment, yaml, definition.hash); await install(saved); await onSaved() } catch (error) { report(error) } finally { setSaving(false) } }
  return <section className="authored-connections" aria-label="Desired connections"><div className="patch-toolbar"><div><h2>Desired group membership (authored state)</h2><p className="muted">Edit each complete ordered member list. An instance may appear in at most one group.</p></div><button className="primary" disabled={!changed.length || saving} onClick={() => void save()}>{saving ? 'Saving…' : 'Save memberships'}</button></div><PlannerWarnings warnings={warnings} />{Object.keys(groups).length ? <table><caption>Complete ordered group membership</caption><thead><tr><th scope="col">Group</th><th scope="col">Members, in priority order</th></tr></thead><tbody>{Object.keys(groups).map((group) => <tr key={group}><th scope="row">{group}</th><td><textarea aria-label={`Members for ${group}`} value={(draft[group] ?? []).join('\n')} onChange={(event) => setDraft((current) => ({ ...current, [group]: event.target.value.split(/[\n,]/).map((member) => member.trim()).filter(Boolean) }))} /></td></tr>)}</tbody></table> : <p>No authored groups.</p>}</section>
}

export function RoutingEditor({ client, deployment, onSaved, onOperation, report }: { client: ApiClient; deployment: string; onSaved: () => Promise<void>; onOperation: (operation: Operation) => void; report: (error: unknown) => void }) {
  const [definition, setDefinition] = useState<DeploymentDefinition | null>(null); const [draft, setDraft] = useState(''); const [validated, setValidated] = useState(false); const [diagnostics, setDiagnostics] = useState<string[]>([]); const [warnings, setWarnings] = useState<PlannerWarning[]>([]); const [followUp, setFollowUp] = useState<'none' | 'plan' | 'apply'>('none')
  const load = async () => { try { const found = await client.definition(deployment); setDefinition(found); setDraft(found.yaml); setValidated(false); setWarnings([]) } catch (error) { report(error) } }
  if (!definition) return <section className="routing-panel"><h2>Routing</h2><p>Custom domains, browser identity routes, and managed profiles are edited in the authored definition.</p><button onClick={load}>Load routing definition</button></section>
  const changed = draft !== definition.yaml; const diff = lineDiff(definition.yaml, draft)
  const validate = async () => { try { const result = await client.validateDeployment(deployment, draft); setDiagnostics(result.diagnostics.map((item) => `${item.code} at ${item.path}: ${item.message}`)); setWarnings(result.warnings ?? []); setValidated(true) } catch (error) { setValidated(false); setWarnings([]); report(error) } }
  const save = async () => { try { await client.updateDefinitionValidated(deployment, draft, definition.hash); setValidated(false); if (followUp !== 'none') onOperation(await client.command(followUp, `deployments/${deployment}.yaml`)); await load(); await onSaved() } catch (error) { report(error) } }
  return <section className="routing-panel"><h2>Routing</h2><p>Edit group and instance <code>address</code> fields, host-router listeners, and <code>managedProfiles</code>. Every save validates the complete deployment first.</p><label>Deployment YAML<textarea className="yaml-editor" value={draft} onChange={(event) => { setDraft(event.target.value); setValidated(false); setWarnings([]) }} /></label>{changed && <><h3>Full YAML diff</h3><pre className="yaml-diff">{diff}</pre></>}{diagnostics.length > 0 && <ul>{diagnostics.map((diagnostic) => <li key={diagnostic}>{diagnostic}</li>)}</ul>}<PlannerWarnings warnings={warnings} /><label>After saving<select value={followUp} onChange={(event) => setFollowUp(event.target.value as typeof followUp)}><option value="none">Do not reconcile yet</option><option value="plan">Plan</option><option value="apply">Plan and Up</option></select></label><div><button disabled={!changed} onClick={validate}>Validate changes</button><button className="primary" disabled={!changed || !validated} onClick={save}>Apply definition edit</button></div></section>
}

function lineDiff(oldText: string, newText: string) { const oldLines = oldText.split('\n'); const newLines = newText.split('\n'); const output: string[] = []; const length = Math.max(oldLines.length, newLines.length); for (let index = 0; index < length; index += 1) { if (oldLines[index] === newLines[index]) output.push(`  ${oldLines[index] ?? ''}`); else { if (oldLines[index] !== undefined) output.push(`- ${oldLines[index]}`); if (newLines[index] !== undefined) output.push(`+ ${newLines[index]}`) } } return output.join('\n') }
