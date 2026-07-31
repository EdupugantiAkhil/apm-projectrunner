import { useEffect, useMemo, useState } from 'react'
import type { AdapterRecord, ApiClient, DeploymentValidation, DeviceRecord, JsonSchema, Operation, ProfileDefinition, ProfileRecord, ProfileValidation, SourceRecord } from './api'
import SchemaForm from './SchemaForm'

const namePattern = /^[a-z](?:[a-z0-9-]{0,61}[a-z0-9])?$/
const profileKey = (profile: ProfileRecord) => `${profile.deployment}:${profile.origin.kind}:${'source' in profile.origin ? profile.origin.source : 'project'}:${profile.name}`
const trusted = (profile: ProfileRecord) => profile.trust === 'trusted' || profile.trust === 'imported'

export default function DeploymentBuilder({ client, projectRoot = '', sources, profiles = [], devices = [], deployment, close, saved, onOperation, report }: { client: ApiClient; projectRoot?: string; sources: SourceRecord[]; profiles?: ProfileRecord[]; devices?: DeviceRecord[]; deployment?: string; close: () => void; saved: (name: string) => Promise<void>; onOperation: (operation: Operation) => void; report: (error: unknown) => void }) {
  return deployment
    ? <InstanceBuilder client={client} deployment={deployment} sources={sources} profiles={profiles} devices={devices} close={close} saved={saved} />
    : <NewDeploymentBuilder client={client} projectRoot={projectRoot} sources={sources} close={close} saved={saved} onOperation={onOperation} report={report} />
}

function InstanceBuilder({ client, deployment, sources, profiles, devices, close, saved }: { client: ApiClient; deployment: string; sources: SourceRecord[]; profiles: ProfileRecord[]; devices: DeviceRecord[]; close: () => void; saved: (name: string) => Promise<void> }) {
  const [definition, setDefinition] = useState<{ hash: string } | null>(null); const [checkout, setCheckout] = useState(''); const [profileId, setProfileId] = useState(''); const [profileDefinition, setProfileDefinition] = useState<ProfileDefinition | null>(null); const [device, setDevice] = useState(''); const [instance, setInstance] = useState(''); const [parameters, setParameters] = useState<Record<string, unknown>>({}); const [parametersValid, setParametersValid] = useState(true); const [availability, setAvailability] = useState<Record<string, { valid: boolean; reason: string }>>({}); const [filtering, setFiltering] = useState(false); const [validation, setValidation] = useState<ProfileValidation | null>(null); const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({}); const [busy, setBusy] = useState(''); const [announcement, setAnnouncement] = useState('Instance form ready')
  const chosen = profiles.find((profile) => profileKey(profile) === profileId) ?? null
  const selectable = profiles.filter((profile) => trusted(profile) && availability[profileKey(profile)]?.valid)
  const unavailable = profiles.filter((profile) => !trusted(profile) || availability[profileKey(profile)]?.valid === false)
  const parameterSchema = useMemo(() => schemaForParameters(profileDefinition), [profileDefinition])
  useEffect(() => { void client.definition(deployment).then((found) => setDefinition({ hash: found.hash })).catch((error) => setFieldErrors({ profile: String(error) })) }, [client, deployment])
  useEffect(() => {
    setProfileId(''); setProfileDefinition(null); setParameters({}); setValidation(null); setAvailability({}); setFieldErrors({})
    if (!checkout) return
    let current = true; setFiltering(true)
    const candidates = profiles.filter(trusted)
    void Promise.all(candidates.map(async (profile) => {
      try { const result = await client.validateProfile(profile, checkout, { targetDeployment: deployment }); return [profileKey(profile), { valid: result.valid, reason: result.error ?? result.diagnostics[0]?.message ?? (result.valid ? 'Valid for checkout' : 'Profile validation failed') }] as const }
      catch (error) { return [profileKey(profile), { valid: false, reason: String(error) }] as const }
    })).then((entries) => { if (current) setAvailability(Object.fromEntries(entries)) }).finally(() => { if (current) setFiltering(false) })
    return () => { current = false }
  }, [checkout, client, deployment, profiles])
  useEffect(() => {
    setProfileDefinition(null); setParameters({}); setValidation(null); setFieldErrors((current) => ({ ...current, profile: '' }))
    if (!chosen) return
    let current = true
    void client.profile(chosen.name, chosen.deployment, chosen.origin).then((found) => { if (!current) return; setProfileDefinition(found); setParameters(parameterDefaults(found)) }).catch((error) => { if (current) setFieldErrors((errors) => ({ ...errors, profile: String(error) })) })
    return () => { current = false }
  }, [chosen, client])
  useEffect(() => {
    setValidation(null)
    if (!chosen || !checkout || !device || !namePattern.test(instance) || !profileDefinition || !parametersValid) return
    let current = true
    const timer = window.setTimeout(() => {
      const values = Object.fromEntries(Object.entries(parameters).map(([name, value]) => [name, String(value ?? '')]))
      void client.validateProfile(chosen, checkout, { targetDeployment: deployment, instanceName: instance, device, parameters: values }).then((result) => {
        if (!current) return
        setValidation(result); setFieldErrors(diagnosticFields(result)); setAnnouncement(result.valid ? 'Instance preview validation passed' : 'Instance preview has field errors')
      }).catch((error) => { if (current) { setValidation(null); setFieldErrors((errors) => ({ ...errors, profile: String(error) })); setAnnouncement('Instance preview validation failed') } })
    }, 300)
    return () => { current = false; window.clearTimeout(timer) }
  }, [chosen, checkout, client, deployment, device, instance, parameters, parametersValid, profileDefinition])
  const save = async () => {
    if (!definition || !validation?.valid || !validation.draft) return
    setBusy('save')
    try { await client.updateDefinition(deployment, validation.draft, definition.hash); setAnnouncement(`Instance ${instance} appended`); await saved(deployment) } catch (error) { setFieldErrors((errors) => ({ ...errors, profile: String(error) })) } finally { setBusy('') }
  }
  const parameterErrors = Object.fromEntries(Object.entries(fieldErrors).filter(([name]) => name.startsWith('parameter:')).map(([name, error]) => [name.slice('parameter:'.length), error]))
  return <section className="builder"><div className="title-row"><div><p className="eyebrow">Guided authoring</p><h1>Add instance to {deployment}</h1></div><button onClick={close}>Close builder</button></div><div className="builder-grid"><form onSubmit={(event) => event.preventDefault()}><h2>Instance definition</h2><label>Checkout / worktree<select required value={checkout} onChange={(event) => setCheckout(event.target.value)}><option value="">Choose registered source</option>{sources.map((source) => <option key={source.source.name}>{source.source.name}</option>)}</select><span className="help">Only registered sources are available.</span></label>{checkout && <label>Trusted startup profile<select required aria-invalid={Boolean(fieldErrors.profile)} aria-describedby="instance-profile-error" value={profileId} onChange={(event) => setProfileId(event.target.value)}><option value="">{filtering ? 'Checking profiles…' : 'Choose valid profile'}</option>{selectable.map((profile) => <option key={profileKey(profile)} value={profileKey(profile)}>{profile.name}</option>)}</select></label>}{fieldErrors.profile && <p className="field-error" id="instance-profile-error" role="alert">{fieldErrors.profile}</p>}{checkout && !filtering && selectable.length === 0 && <p className="field-error" id="instance-profile-error">No trusted startup profile is valid for this checkout.</p>}{checkout && unavailable.length > 0 && <details className="unavailable-options"><summary>Unavailable profiles</summary><ul>{unavailable.map((profile) => <li key={profileKey(profile)}><strong>{profile.name}</strong> — {!trusted(profile) ? `${profile.trust}; review/import in Profiles first` : availability[profileKey(profile)]?.reason}</li>)}</ul></details>}{chosen && <><label>Device<select required aria-invalid={Boolean(fieldErrors.device)} aria-describedby="instance-device-error" value={device} onChange={(event) => setDevice(event.target.value)}><option value="">Choose eligible device</option>{devices.map((item) => <option key={item.name} value={item.name} disabled={item.eligibility !== 'eligible'}>{item.name}{item.eligibility === 'eligible' ? '' : ` — unavailable: ${item.eligibilityReason}`}</option>)}</select></label>{fieldErrors.device && <p className="field-error" id="instance-device-error" role="alert">{fieldErrors.device}</p>}<ul className="device-choice-notes">{devices.filter((item) => item.eligibility !== 'eligible').map((item) => <li key={item.name}><strong>{item.name}</strong>: {item.eligibilityReason}</li>)}</ul></>}{chosen && device && <label>Instance name<input required pattern="[a-z](?:[a-z0-9-]{0,61}[a-z0-9])?" value={instance} onChange={(event) => setInstance(event.target.value)} /><span className="help">Lowercase DNS label, up to 63 characters.</span></label>}{profileDefinition && device && <SchemaForm schema={parameterSchema} value={parameters} errors={parameterErrors} onChange={(value, valid) => { setParameters(value); setParametersValid(valid) }} />}</form><section><h2>Live expansion preview</h2>{validation ? <>{validation.expandedServices.length > 0 ? <ul>{validation.expandedServices.map((service) => <li className="mono" key={service}>{service}</li>)}</ul> : <p className="muted">No services expand from this profile.</p>}<div className="service-preview">{validation.services.map((service) => <article key={service.name}><h3>{service.name}</h3><dl><dt>Ports</dt><dd>{service.ports.length ? service.ports.join(', ') : 'none'}</dd><dt>Volumes</dt><dd>{service.volumes.length ? service.volumes.map((volume) => `${volume.name} → ${volume.target}${volume.readOnly ? ' (read-only)' : ''}`).join(', ') : 'none'}</dd></dl></article>)}</div>{validation.error && <p className="field-error">{validation.error}</p>}</> : <p className="muted">Choose a checkout, valid trusted profile, eligible device, name, and parameters to preview planner expansion.</p>}<button className="primary" disabled={!validation?.valid || !validation.draft || busy === 'save'} onClick={() => void save()}>{busy === 'save' ? 'Appending…' : 'Append instance'}</button></section></div><div className="sr-only" aria-live="polite">{announcement}</div></section>
}

function schemaForParameters(profile: ProfileDefinition | null): JsonSchema {
  const parameters = profile?.definition.parameters
  if (!parameters || typeof parameters !== 'object' || Array.isArray(parameters)) return { type: 'object', title: 'Profile parameters', properties: {} }
  const entries = Object.entries(parameters as Record<string, unknown>); const properties: Record<string, JsonSchema> = {}; const required: string[] = []
  for (const [name, value] of entries) { const spec = value && typeof value === 'object' ? value as Record<string, unknown> : {}; properties[name] = { type: 'string', title: name, ...(typeof spec.default === 'string' ? { default: spec.default } : {}) }; if (spec.required === true) required.push(name) }
  return { type: 'object', title: 'Profile parameters', properties, required }
}

function parameterDefaults(profile: ProfileDefinition) {
  const parameters = profile.definition.parameters
  if (!parameters || typeof parameters !== 'object' || Array.isArray(parameters)) return {}
  return Object.fromEntries(Object.entries(parameters as Record<string, unknown>).map(([name, value]) => { const spec = value && typeof value === 'object' ? value as Record<string, unknown> : {}; return [name, typeof spec.default === 'string' ? spec.default : ''] }))
}

function diagnosticFields(validation: ProfileValidation) {
  const fields: Record<string, string> = {}
  for (const diagnostic of validation.diagnostics) {
    if (diagnostic.path.endsWith('.block')) fields.profile = diagnostic.message
    else if (diagnostic.path.endsWith('.device') || /^remote /i.test(diagnostic.message)) fields.device = diagnostic.message
    else { const parameter = diagnostic.path.split('.parameters.')[1]; if (parameter) fields[`parameter:${parameter}`] = diagnostic.message }
  }
  if (validation.error) fields.profile = validation.error
  return fields
}

function NewDeploymentBuilder({ client, projectRoot, sources, close, saved, onOperation, report }: { client: ApiClient; projectRoot: string; sources: SourceRecord[]; close: () => void; saved: (name: string) => Promise<void>; onOperation: (operation: Operation) => void; report: (error: unknown) => void }) {
  const [name, setName] = useState(''); const [instance, setInstance] = useState(''); const [block, setBlock] = useState(''); const [source, setSource] = useState(''); const [adapters, setAdapters] = useState<AdapterRecord[]>([]); const [adapter, setAdapter] = useState(''); const [configuration, setConfiguration] = useState<Record<string, unknown>>({}); const [formValid, setFormValid] = useState(true); const [validation, setValidation] = useState<DeploymentValidation | null>(null); const [announcement, setAnnouncement] = useState('Builder ready'); const [startAfterSave, setStartAfterSave] = useState(false)
  useEffect(() => { void client.adapters().then(setAdapters).catch(report) }, [])
  const chosen = adapters.find((item) => `${item.kind}:${item.declaration.id ?? ''}` === adapter) ?? adapters[0]
  useEffect(() => { if (chosen && !adapter) setAdapter(`${chosen.kind}:${chosen.declaration.id ?? ''}`) }, [chosen, adapter])
  const deployableSources = sources.filter((item) => deploymentSource(item, projectRoot) !== null)
  const yaml = useMemo(() => buildYaml(name, instance, block, source, deployableSources, projectRoot, configuration), [name, instance, block, source, deployableSources, projectRoot, configuration])
  useEffect(() => { setValidation(null) }, [yaml])
  useEffect(() => {
    if (!namePattern.test(name) || !instance || !block || !source || !formValid) return
    const timer = window.setTimeout(() => { void client.validateDeployment(name, yaml).then((result) => { setValidation(result); setAnnouncement('Draft validation passed') }).catch((error) => { setValidation(null); setAnnouncement('Draft validation failed'); report(error) }) }, 350)
    return () => window.clearTimeout(timer)
  }, [name, instance, block, source, formValid, yaml])
  const validate = async () => { try { const result = await client.validateDeployment(name, yaml); setValidation(result); setAnnouncement(`Draft valid: ${String(result.preview.expandedServiceCount ?? 0)} expanded services`) } catch (error) { setValidation(null); setAnnouncement('Draft validation failed'); report(error) } }
  const save = async () => { try { await client.createDeployment(name, yaml); setAnnouncement(`Deployment ${name} saved`); if (startAfterSave) onOperation(await client.command('apply', `deployments/${name}.yaml`)); await saved(name) } catch (error) { report(error) } }
  return <section className="builder"><div className="title-row"><div><p className="eyebrow">Creation flow</p><h1>New deployment</h1></div><button onClick={close}>Close builder</button></div><div className="builder-grid"><form onSubmit={(event) => { event.preventDefault(); void validate() }}><h2>Definition</h2><label>Name<input required pattern="[a-z](?:[a-z0-9-]{0,61}[a-z0-9])?" value={name} onChange={(event) => setName(event.target.value)} /><span className="help">Lowercase DNS label, up to 63 characters.</span></label><label>Instance name<input required value={instance} onChange={(event) => setInstance(event.target.value)} /></label><label>Block name<input required value={block} onChange={(event) => setBlock(event.target.value)} /></label><label>Source<select required value={source} onChange={(event) => setSource(event.target.value)}><option value="">Choose project-local worktree</option>{deployableSources.map((item) => <option key={item.source.name}>{item.source.name}</option>)}</select><span className="help">Repository clones and worktrees outside this project cannot be deployment sources.</span></label><label>Execution adapter<select value={adapter} onChange={(event) => { setAdapter(event.target.value); setConfiguration({}) }}>{adapters.map((item) => <option key={`${item.kind}:${item.declaration.id ?? ''}`} value={`${item.kind}:${item.declaration.id ?? ''}`}>{item.kind} · {String(item.declaration.id ?? 'registered')}</option>)}</select></label>{chosen && <SchemaForm schema={chosen.configurationSchema} value={configuration} onChange={(value, valid) => { setConfiguration(value); setFormValid(valid) }} />}<button disabled={!namePattern.test(name) || !instance || !block || !source || !formValid}>Validate draft</button></form><section><h2>Draft YAML</h2><pre className="draft-yaml">{yaml}</pre><h2>Plan preview</h2>{validation ? <><dl><dt>Expanded services</dt><dd>{String(validation.preview.expandedServiceCount ?? 0)}</dd><dt>Images/builds, ports, volumes</dt><dd>See generated Compose preview below.</dd><dt>Routes</dt><dd>{JSON.stringify(validation.preview.routes ?? [])}</dd></dl><PlannerWarnings warnings={validation.warnings} /></> : <p className="muted">Validate to derive expanded resources and routes.</p>}<label className="check"><input type="checkbox" checked={startAfterSave} onChange={(event) => setStartAfterSave(event.target.checked)} />Run Up after saving</label><button className="primary" disabled={!validation?.valid} onClick={save}>Save deployment</button></section></div><div className="sr-only" aria-live="polite">{announcement}</div></section>
}

function PlannerWarnings({ warnings }: { warnings: DeploymentValidation['warnings'] }) {
  if (!warnings?.length) return null
  return <aside className="planner-warnings" aria-label="Planner warnings"><h3>Warnings</h3><ul>{warnings.map((warning) => <li key={`${warning.code}:${warning.path}:${warning.message}`}><strong>{warning.code}</strong> at <code>{warning.path}</code>: {warning.message}</li>)}</ul></aside>
}

function deploymentSource(source: SourceRecord, projectRoot: string) {
  const repository = source.inspection.identity.repository
  const ref = source.source.requestedRef ?? source.inspection.identity.ref
  const prefix = projectRoot.endsWith('/') ? projectRoot : `${projectRoot}/`
  if (source.inspection.linkedWorktree !== true || !repository || repository === source.source.path || !ref || !source.source.path.startsWith(prefix)) return null
  return { repository, ref, path: source.source.path.slice(prefix.length) }
}

function buildYaml(name: string, instance: string, block: string, sourceName: string, sources: SourceRecord[], projectRoot: string, configuration: Record<string, unknown>) {
  const source = sources.find((item) => item.source.name === sourceName)
  const checkout = source ? deploymentSource(source, projectRoot) : null
  const execution = Object.keys(configuration).length ? configuration : { type: 'container', image: 'replace-me:local' }
  return JSON.stringify({ apiVersion: 'switchyard.dev/v1alpha2', kind: 'Deployment', metadata: { name }, spec: { repositories: sourceName && checkout ? { [sourceName]: { clone: checkout.repository } } : {}, sources: sourceName && checkout ? { [sourceName]: { repository: sourceName, ref: checkout.ref, path: checkout.path } } : {}, blocks: block ? { [block]: { services: { main: { execution } } } } : {}, instances: instance && block && sourceName ? [{ name: instance, block, source: sourceName, parameters: {} }] : [], groups: {}, bindings: {}, routes: {}, managedProfiles: {} } }, null, 2)
}

export function BlockLibrary({ adapters }: { adapters: AdapterRecord[] }) {
  return <section><h1>Block library</h1><p>Registered execution and probe adapters. Forms are generated directly from each draft 2020-12 configuration schema.</p><div className="adapter-list">{adapters.map((adapter) => <article key={`${adapter.kind}:${adapter.declaration.id ?? ''}`}><h2>{String(adapter.declaration.id ?? adapter.kind)}</h2><p>{adapter.kind} · v{String(adapter.declaration.version ?? 'unknown')}</p><p>Capabilities: {Array.isArray(adapter.declaration.capabilities) ? adapter.declaration.capabilities.join(', ') : 'declared by adapter'}</p><SchemaForm readOnly schema={adapter.configurationSchema} /></article>)}</div></section>
}
