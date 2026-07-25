import { useState, type FormEvent } from 'react'
import type { ApiClient, DeploymentSummary, Operation, RunAction, RunActionPreview, RunActionsResponse, StructuredRunActionInput } from './api'

const lines = (value: FormDataEntryValue | null) => String(value ?? '').split('\n').map((line) => line.trim()).filter(Boolean)
const executionText = (preview: RunActionPreview) => preview.execution.type === 'structured' ? preview.execution.argv.join(' ') : preview.execution.command
const targetText = (preview: RunActionPreview) => preview.target.kind === 'deployment' ? `${preview.target.name} (${preview.target.bundle})` : preview.target.root

export default function RunActionsView({ client, response, deployments, reload, startOperation, report }: { client: ApiClient; response: RunActionsResponse | null; deployments: DeploymentSummary[]; reload: () => Promise<void>; startOperation: (operation: Operation) => void; report: (error: unknown) => void }) {
  const [editing, setEditing] = useState<Extract<RunAction, { type: 'structured' }> | null>(null)
  const [targets, setTargets] = useState<Record<string, string>>({})
  const [preview, setPreview] = useState<RunActionPreview | null>(null)
  const [busy, setBusy] = useState('')

  const save = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const form = event.currentTarget; const data = new FormData(form)
    const action: StructuredRunActionInput = {
      name: String(data.get('name')).trim(), description: String(data.get('description')).trim() || undefined,
      type: 'structured', command: String(data.get('command')) as StructuredRunActionInput['command'],
      overlays: lines(data.get('overlays')), variation: String(data.get('variation')).trim() || undefined, set: lines(data.get('set')),
    }
    setBusy('save')
    try { if (editing) await client.updateRunAction(editing.name, action); else await client.createRunAction(action); setEditing(null); form.reset(); await reload() } catch (error) { report(error) } finally { setBusy('') }
  }
  const remove = async (action: Extract<RunAction, { type: 'structured' }>) => {
    if (!window.confirm(`Delete structured run action ${action.name}?`)) return
    setBusy(`delete:${action.name}`)
    try { await client.deleteRunAction(action.name); if (editing?.name === action.name) setEditing(null); await reload() } catch (error) { report(error) } finally { setBusy('') }
  }
  const inspectExecution = async (action: RunAction) => {
    const deployment = targets[action.name] || deployments[0]?.name
    if (action.type === 'structured' && !deployment) return
    setBusy(`preview:${action.name}`)
    try { setPreview(await client.previewRunAction(action.name, action.type === 'structured' ? `.switchyard/generated/${deployment}/resolved-deployment.yaml` : undefined)) } catch (error) { report(error) } finally { setBusy('') }
  }
  const execute = async () => {
    if (!preview) return
    setBusy('execute')
    try { const operation = await client.executeRunAction(preview.name, preview, preview.shellAcknowledgementRequired); setPreview(null); startOperation(operation) } catch (error) { report(error) } finally { setBusy('') }
  }

  return <section><div className="title-row"><div><p className="eyebrow">Project operations</p><h1>Run actions</h1></div></div>
    <section className="profile-boundary" role="note"><h2>Shell action authoring is unavailable in the browser</h2><p>Create and edit shell actions through the CLI or TUI. The browser may list and run an existing shell action only after showing its exact command and satisfying the project-local acknowledgement gate.</p></section>
    {!response ? <p role="status">Loading run actions…</p> : response.actions.length === 0 ? <p>No project run actions are defined.</p> : <div className="profile-grid">{response.actions.map((action) => <article className="profile-card" key={action.name}><header><div><h2>{action.name}</h2><p><span className="kind-label">{action.type}</span> {action.description ?? 'No description.'}</p></div></header>
      {action.type === 'structured' ? <><p><span className="mono">{action.command}</span>{(action.overlays ?? []).length ? ` · ${(action.overlays ?? []).length} overlay(s)` : ''}{action.variation ? ` · variation ${action.variation}` : ''}</p><label>Deployment target<select aria-label={`Deployment target for ${action.name}`} value={targets[action.name] ?? deployments[0]?.name ?? ''} onChange={(event) => setTargets((current) => ({ ...current, [action.name]: event.target.value }))}><option value="">Choose deployment</option>{deployments.map((deployment) => <option key={deployment.name}>{deployment.name}</option>)}</select></label><div className="profile-actions"><button onClick={() => setEditing(action)}>Edit</button><button className="danger" disabled={busy === `delete:${action.name}`} onClick={() => void remove(action)}>Delete</button><button className="primary" disabled={!deployments.length || busy === `preview:${action.name}`} onClick={() => void inspectExecution(action)}>Preview and run</button></div></> : <><pre>{action.command}</pre><div className="profile-actions"><button className="primary" disabled={busy === `preview:${action.name}`} onClick={() => void inspectExecution(action)}>Preview and run shell action</button></div></>}
    </article>)}</div>}
    <form className="device-form" key={editing?.name ?? 'new'} onSubmit={save}><h2>{editing ? `Edit structured action · ${editing.name}` : 'Create structured action'}</h2><label>Name<input required name="name" defaultValue={editing?.name} /></label><label>Description<input name="description" defaultValue={editing?.description ?? ''} /></label><label>Command<select name="command" defaultValue={editing?.command ?? 'plan'}><option value="up">up</option><option value="down">down</option><option value="plan">plan</option><option value="status">status</option></select></label><label>Overlays <span className="muted">(one path per line)</span><textarea name="overlays" defaultValue={editing?.overlays?.join('\n') ?? ''} /></label><label>Variation <span className="muted">(optional)</span><input name="variation" defaultValue={editing?.variation ?? ''} /></label><label>Set values <span className="muted">(one KEY=VALUE per line)</span><textarea name="set" defaultValue={editing?.set?.join('\n') ?? ''} /></label><div><button type="button" disabled={!editing} onClick={() => setEditing(null)}>Cancel edit</button><button className="primary" disabled={busy === 'save'}>{editing ? 'Save structured action' : 'Create structured action'}</button></div></form>
    {preview && <div className="modal-backdrop"><div role="dialog" aria-modal="true" aria-labelledby="run-action-preview-title" className="modal"><h2 id="run-action-preview-title">Confirm run action · {preview.name}</h2><p>{preview.description}</p><dl><dt>Target</dt><dd className="mono">{targetText(preview)}</dd><dt>{preview.execution.type === 'structured' ? 'Argument vector' : 'Shell command'}</dt><dd><pre>{executionText(preview)}</pre></dd></dl>{preview.shellAcknowledgementRequired && <p className="warning"><strong>Shell execution acknowledgement:</strong> this arbitrary command runs in the project directory with your user permissions. Confirming records the existing project-local acknowledgement before execution.</p>}<div><button onClick={() => setPreview(null)}>Cancel</button><button className="primary" disabled={busy === 'execute'} onClick={() => void execute()}>{preview.shellAcknowledgementRequired ? 'Acknowledge and run' : 'Confirm and run'}</button></div></div></div>}
  </section>
}
