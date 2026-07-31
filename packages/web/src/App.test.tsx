import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import App from './App'
import { ApiClient, type DeviceRecord, type Operation, type OperationEvent, type RunActionPreview, type RunActionsResponse, type RouteState, type SourceRecord } from './api'

const connectionBlocks = {
  java: { services: { app: { provides: { java: {} } } } },
  python: { services: { app: { provides: { python: {} } } } },
  database: { services: { app: { provides: { database: {} } } } },
}
const deployment = {
  apiVersion: 'v1', deployment: 'comparison', definitionHash: 'definition123', resourceHash: 'resource123', appliedAt: 1,
  snapshot: { spec: { instances: [{ name: 'ui-feature', device: 'build-host' }, { name: 'backend-a', block: 'java' }, { name: 'backend-b', block: 'java' }, { name: 'python-a', block: 'python' }, { name: 'python-b', block: 'python' }, { name: 'shared-db', block: 'database' }], blocks: connectionBlocks, bindings: { 'ui-feature': 'feature' }, routes: { 'ui-feature': { java: 'backend-a', python: 'python-a', database: 'shared-db' } }, groups: { base: { instances: ['backend-b', 'python-b', 'shared-db'] }, feature: { address: 'ui.comparison.localhost', instances: ['backend-a', 'python-a', 'shared-db'] } } } }, manifest: {},
  sourceIdentities: { 'ui-feature': { path: '/worktrees/ui-a', ref: 'feature/ui-redesign', commit: '35ad2abcdef', dirty: true } },
  reconciliation: { deployment: 'comparison', diagnostics: [] }, resources: [{ kind: 'container', id: 'one', name: 'comparison-ui-feature', labels: { 'dev.switchyard.instance': 'ui-feature' }, state: 'healthy', device: 'build-host' }],
  customDomains: ['ui.comparison.localhost'], bindings: { 'ui-feature': 'feature' },
}
const source: SourceRecord = { source: { name: 'feature-ui', kind: 'managed', path: '/worktrees/ui-a' }, inspection: { identity: { path: '/worktrees/ui-a', ref: 'feature/ui-redesign', commit: '35ad2abcdef', dirty: true }, branch: 'feature/ui-redesign', changes: { staged: 1, unstaged: 2, untracked: 3 }, ahead: 2, behind: 0, unknownCode: null } }
const unmanagedSource = { source: { name: 'shared-app', kind: 'unmanaged' as const, path: '/code/shared-app' }, inspection: { identity: { path: '/code/shared-app', ref: 'main', commit: '123456789ab', dirty: true }, branch: 'main', changes: { staged: 4, unstaged: 5, untracked: 6 }, ahead: 0, behind: 0, unknownCode: null } }
const sourceProfile = { apiVersion: 'v1', name: 'api', deployment: 'comparison', origin: { kind: 'discovered-in-source' as const, source: 'feature-ui', commit: '35ad2abcdef' }, trust: 'not-imported' as const, shadowed: false, services: [{ name: 'web', adapterKind: 'container' as const }] }
const trustedProfile = { apiVersion: 'v1', name: 'worker-profile', deployment: 'comparison', origin: { kind: 'project' as const }, trust: 'trusted' as const, shadowed: false, services: [{ name: 'web', adapterKind: 'container' as const }] }
const localDevice: DeviceRecord = { name: 'local', kind: 'local', host: null, port: null, user: null, identityFile: null, createdAt: null, lastCheckedAt: null, lastCheckStatus: 'eligible', lastCheckDetail: null, reachability: 'reachable', eligibility: 'eligible', eligibilityReason: 'local execution is always eligible', placedInstances: [] }
const remoteDevice: DeviceRecord = { name: 'build-host', kind: 'ssh', host: 'host.test', port: 22, user: 'dev', identityFile: null, createdAt: 1, lastCheckedAt: null, lastCheckStatus: 'never', lastCheckDetail: null, reachability: 'unchecked', eligibility: 'ineligible', eligibilityReason: 'unchecked', placedInstances: [{ deployment: 'comparison', instance: 'ui-feature' }] }
const authoredSpec = { instances: [{ name: 'ui-feature', block: 'web-ui' }, { name: 'ui-unbound', block: 'web-ui' }, { name: 'backend-a', block: 'java' }, { name: 'backend-b', block: 'java' }, { name: 'python-a', block: 'python' }, { name: 'python-b', block: 'python' }, { name: 'shared-db', block: 'database' }], blocks: { ...connectionBlocks, 'web-ui': { services: { web: { consumes: { java: { protocol: 'tcp' }, python: { protocol: 'grpc' }, database: { protocol: 'tcp' } } } } } }, groups: { base: { instances: ['backend-b', 'python-b', 'shared-db'] }, feature: { instances: ['backend-a', 'python-a', 'shared-db'] } }, bindings: { 'ui-feature': 'feature' } }
const authoredYaml = 'apiVersion: switchyard.dev/v1alpha2\nkind: Deployment\nmetadata:\n  name: comparison\nspec:\n  instances: []\n  blocks: {}\n  groups: {}\n  bindings:\n    ui-feature: feature\n'
const authoredDefinition = { apiVersion: 'v1', name: 'comparison', path: '/project/deployments/comparison.yaml', hash: 'hash-one', yaml: authoredYaml }
const routeState: RouteState = {
  apiVersion: 'v1', deployment: 'comparison', bindings: [{ router: 'host', binding: 'ui-feature', desiredVersion: 5, desiredChecksum: 'desired-five', currentVersion: 4, currentChecksum: 'current-four', previousVersion: 3, previousChecksum: 'previous-three', observedVersion: 4, observedChecksum: 'observed-four', status: 'pending', transition: { state: 'draining', strategy: 'drain', timeoutMs: 2500 }, lastErrorCode: null, updatedAt: 1700000000300 }],
  history: [{ sequence: 1, router: 'host', binding: 'ui-feature', operationId: 'op-old', version: 3, checksum: 'previous-three', activationStatus: 'active', recordedAt: 1700000000100, context: {} }, { sequence: 2, router: 'host', binding: 'ui-feature', operationId: 'op-rollback', version: 4, checksum: 'current-four', activationStatus: 'rolled_back', recordedAt: 1700000000200, context: { errorCode: 'provider_unhealthy' } }],
}

class MockEventSource extends EventTarget {
  static instances: MockEventSource[] = []
  onerror: ((event: Event) => void) | null = null
  readonly url: string
  constructor(url: string) { super(); this.url = url; MockEventSource.instances.push(this) }
  close() {}
  emit(event: OperationEvent) { this.dispatchEvent(new MessageEvent(event.kind, { data: JSON.stringify(event), lastEventId: String(event.id) })) }
}

function json(value: unknown, status = 200) { return new Response(JSON.stringify(value), { status, headers: { 'content-type': 'application/json' } }) }
function installFetch() {
  let operationReads = 0
  let deviceStatus = 'never'
  const fetchMock = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input)
    if (url.endsWith('/project')) return json({ apiVersion: 'v1', name: 'payments-lab', root: '/project', registered: true })
    if (url.endsWith('/deployments') && (!init?.method || init.method === 'GET')) return json({ apiVersion: 'v1', deployments: [{ name: 'comparison', definitionHash: 'definition123', resourceHash: 'resource123', appliedAt: 1, lastOperation: { id: 'old', kind: 'apply', status: 'succeeded', startedAt: 1, finishedAt: 2 }, customDomains: [], bindings: {} }] })
    if (url.endsWith('/deployments/comparison/routes')) return json(routeState)
    if (url.endsWith('/deployments/comparison')) return json(deployment)
    if (url.endsWith('/sources')) return json([source])
    if (url.endsWith('/devices/build-host/check') && init?.method === 'POST') { deviceStatus = 'eligible'; return json({ ...remoteDevice, lastCheckedAt: 1000, lastCheckStatus: 'eligible', lastCheckDetail: 'eligible for remote container execution (docker 28.5.1)', reachability: 'reachable', eligibility: 'eligible', eligibilityReason: 'eligible for remote container execution (docker 28.5.1)' }) }
    if (url.endsWith('/devices') && init?.method === 'POST') return json({ ...remoteDevice, ...JSON.parse(String(init.body)), placedInstances: [] }, 201)
    if (url.endsWith('/devices')) return json([localDevice, deviceStatus === 'eligible' ? { ...remoteDevice, lastCheckedAt: 1000, lastCheckStatus: 'eligible', lastCheckDetail: 'eligible for remote container execution (docker 28.5.1)', reachability: 'reachable', eligibility: 'eligible', eligibilityReason: 'eligible for remote container execution (docker 28.5.1)' } : remoteDevice])
    if (url.endsWith('/adapters')) return json([{ kind: 'execution', declaration: { id: 'container', version: '1', capabilities: ['container'] }, configurationSchema: { type: 'object', properties: { type: { type: 'string', enum: ['container'], default: 'container' }, image: { type: 'string' } } } }])
    if (url.endsWith('/profiles')) return json({ apiVersion: 'v1', profiles: [sourceProfile], sourceErrors: [] })
    if (url.includes('/profiles/api?')) return json({ ...sourceProfile, definition: { parameters: { LOG_LEVEL: { default: 'info' } }, services: { web: { execution: { type: 'container', image: 'busybox' } } } } })
    if (url.includes('/profiles/api/manifest?')) return json({ apiVersion: 'v1', source: 'feature-ui', manifest: 'version: 1\nprofiles:\n  api: {}\n', reviewHash: 'review-one' })
    if (url.endsWith('/profiles/api/import') && init?.method === 'POST') return json({ ...sourceProfile, origin: { kind: 'imported-from-source', source: 'feature-ui', commit: '35ad2abcdef' }, trust: 'imported' }, 201)
    if (url.endsWith('/profiles/api/validate') && init?.method === 'POST') return json({ apiVersion: 'v1', name: 'api', deployment: 'comparison', checkout: 'feature-ui', valid: true, expandedServices: ['comparison--profile-validation-preview--web'], diagnostics: [], error: null })
    if (url.includes('/operations?deployment=comparison&instance=')) return json({ apiVersion: 'v1', operations: [], nextCursor: null })
    if (url.endsWith('/operations')) return json({ apiVersion: 'v1', operations: [{ apiVersion: 'v1', id: 'op-cli', deployment: 'comparison', instance: null, kind: 'cleanup', destructive: true, status: 'succeeded', startedAt: 5, finishedAt: 6, error: null, result: null }], nextCursor: null })
    if (url.endsWith('/deployments/comparison/definition') && (!init?.method || init.method === 'GET')) return json({ apiVersion: 'v1', name: 'comparison', path: '/project/deployments/comparison.yaml', hash: 'hash-one', yaml: 'metadata:\n  name: comparison\nspec:\n  groups:\n    feature:\n      address: ui.comparison.localhost\n' })
    if (url.endsWith('/deployments/comparison/definition') && init?.method === 'PUT') return json({ apiVersion: 'v1', name: 'comparison', path: '/project/deployments/comparison.yaml', hash: 'hash-two', yaml: JSON.parse(String(init.body)).yaml })
    if (url.endsWith('/deployments') && init?.method === 'POST') { const body = JSON.parse(String(init.body)); if (body.validateOnly) return json({ apiVersion: 'v1', name: body.name, valid: true, diagnostics: [], preview: { expandedServiceCount: 1, routes: ['ui-feature'] } }); return json({ apiVersion: 'v1', name: body.name, path: `/project/deployments/${body.name}.yaml`, hash: 'new-hash', yaml: body.yaml }, 201) }
    if (url.includes('/worktrees/feature-ui')) return json({ staged: 1, unstaged: 2, untracked: 3 })
    if (url.endsWith('/commands/validate') && init?.method === 'POST') return json({ apiVersion: 'v1', id: 'op-new', deployment: 'comparison', instance: null, kind: 'validate', status: 'running', startedAt: 10, finishedAt: null, error: null, result: null }, 202)
    if (url.endsWith('/commands/bind') && init?.method === 'POST') return json({ apiVersion: 'v1', id: 'op-bind', deployment: 'comparison', instance: 'ui-feature', kind: 'bind', status: 'running', startedAt: 10, finishedAt: null, error: null, result: null }, 202)
    if (url.endsWith('/operations/op-new')) { operationReads += 1; return json({ apiVersion: 'v1', id: 'op-new', deployment: 'comparison', instance: null, kind: 'validate', status: 'succeeded', startedAt: 10, finishedAt: 11, error: null, result: { exitCode: 0, stdout: 'valid', stderr: '' } }) }
    if (url.endsWith('/operations/op-bind')) return json({ apiVersion: 'v1', id: 'op-bind', deployment: 'comparison', instance: 'ui-feature', kind: 'bind', status: 'succeeded', startedAt: 10, finishedAt: 11, error: null, result: { exitCode: 0, stdout: 'applied v5', stderr: '' } })
    throw new Error(`unexpected request ${url} (${operationReads})`)
  })
  vi.stubGlobal('fetch', fetchMock)
  return fetchMock
}

describe('Switchyard GUI', () => {
  beforeEach(() => { MockEventSource.instances = []; vi.stubGlobal('EventSource', MockEventSource); installFetch() })
  afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks() })

  it('renders separate route versions, transition state, previous version, and typed rollback history', async () => {
    render(<App client={new ApiClient('test')} />)
    expect(await screen.findByRole('heading', { name: 'comparison', level: 1 })).toBeInTheDocument()
    expect(screen.getByText('payments-lab')).toBeInTheDocument()
    expect(screen.getByText('/worktrees/ui-a')).toBeInTheDocument()
    expect(screen.getByText(/35ad2abcd/)).toBeInTheDocument()
    expect(screen.getByText('healthy')).toBeInTheDocument()
    expect(screen.getAllByText('Authored placement')).toHaveLength(6)
    expect(screen.getAllByText('Observed placement')).toHaveLength(6)
    expect(screen.getAllByText('build-host')).toHaveLength(2)
    expect(screen.getByText('ui.comparison.localhost')).toBeInTheDocument()
    const activeRoutes = screen.getByRole('columnheader', { name: 'Desired' }).closest('table')!; const activeRow = within(activeRoutes).getByRole('cell', { name: 'ui-feature' }).closest('tr')!
    expect(within(activeRow).getByRole('cell', { name: 'v5' })).toBeInTheDocument(); expect(within(activeRow).getByRole('cell', { name: 'v4' })).toBeInTheDocument(); expect(within(activeRow).getByRole('cell', { name: 'v3' })).toBeInTheDocument(); expect(within(activeRow).getByRole('cell', { name: 'draining' })).toBeInTheDocument(); expect(within(activeRow).getByText('rollback recorded at v4 (timestamp 1700000000200)')).toBeInTheDocument()
    const history = screen.getByRole('columnheader', { name: 'Activation' }).closest('table')!; expect(within(history).getByRole('cell', { name: 'rolled_back' })).toBeInTheDocument(); expect(within(history).getByRole('cell', { name: '1700000000200' })).toBeInTheDocument(); expect(within(history).getByRole('cell', { name: 'op-rollback' })).toBeInTheDocument()
  })

  it('shows one honest per-instance inspector with placement, services, connections, and instance-scoped operations', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test'); const inspected = { ...deployment, snapshot: { spec: { ...deployment.snapshot.spec, instances: deployment.snapshot.spec.instances.map((instance) => instance.name === 'ui-feature' ? { ...instance, block: 'web-ui', source: 'feature-ui' } : instance), blocks: { ...deployment.snapshot.spec.blocks, 'web-ui': { services: { web: { consumes: { java: { protocol: 'tcp' }, python: { protocol: 'grpc' }, database: { protocol: 'tcp' } } }, worker: {} } } } } }, resources: [{ kind: 'container', id: 'web', name: 'opaque-runtime-name', labels: { 'dev.switchyard.instance': 'ui-feature', 'dev.switchyard.service': 'web' }, state: 'running (healthy)', device: 'build-host' }, { kind: 'container', id: 'worker', name: 'comparison-ui-feature-worker', labels: {} as Record<string, string>, state: 'running (unhealthy)', device: 'local' }] }
    const instanceOperation: Operation = { apiVersion: 'v1', id: 'op-ui-feature', deployment: 'comparison', instance: 'ui-feature', kind: 'logs', destructive: false, status: 'succeeded', startedAt: 8, finishedAt: 9, error: null, result: null }; vi.spyOn(client, 'deployment').mockResolvedValue(inspected); const operations = vi.spyOn(client, 'operations').mockImplementation(async (filters = {}) => ({ apiVersion: 'v1', operations: filters.instance === 'ui-feature' ? [instanceOperation] : [], nextCursor: null }))
    vi.spyOn(client, 'profiles').mockResolvedValue({ apiVersion: 'v1', profiles: [{ ...trustedProfile, name: 'web-ui' }], sourceErrors: [] })
    render(<App client={client} />); await screen.findByRole('heading', { name: 'comparison', level: 1 }); await user.click(screen.getByRole('button', { name: 'Inspect ui-feature' }))
    expect(screen.getAllByRole('complementary', { name: 'Inspector' })).toHaveLength(1); expect(screen.queryByLabelText('Instance inspector')).not.toBeInTheDocument()
    const inspector = screen.getByRole('complementary', { name: 'Inspector' }); expect(within(inspector).getByRole('heading', { name: 'ui-feature' })).toBeInTheDocument(); expect(await within(inspector).findByText('web-ui · Project · comparison · trusted')).toBeInTheDocument(); expect(within(inspector).getByText('feature-ui')).toBeInTheDocument(); expect(within(inspector).getAllByText('build-host')).toHaveLength(3)
    const web = within(inspector).getByText('web').closest('li')!; expect(within(web).getByText('running (healthy)')).toBeInTheDocument(); expect(within(web).getByText('healthy')).toBeInTheDocument(); expect(within(web).getByText('build-host')).toBeInTheDocument()
    const worker = within(inspector).getByText('worker').closest('li')!; expect(within(worker).getAllByText('not observed')).toHaveLength(3)
    expect(within(inspector).getByText('ui-feature / java → backend-a')).toBeInTheDocument(); expect(within(inspector).getByText('ui-feature / python → python-a')).toBeInTheDocument(); expect(within(inspector).getByText('ui-feature / database → shared-db')).toBeInTheDocument()
    expect(await within(inspector).findByText('op-ui-feature')).toBeInTheDocument(); expect(within(inspector).getByText(/Only operations durably attributed to instance/)).toBeInTheDocument(); expect(within(inspector).getByText(/Deployment-wide operations and legacy records whose instance is null are not blended/)).toBeInTheDocument(); expect(operations).toHaveBeenCalledWith({ deployment: 'comparison', instance: 'ui-feature' })
  })

  it('names a startup profile the library does not list without inventing provenance', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test'); const inspected = { ...deployment, snapshot: { spec: { ...deployment.snapshot.spec, instances: deployment.snapshot.spec.instances.map((instance) => instance.name === 'ui-feature' ? { ...instance, block: 'web-ui' } : instance), blocks: { ...deployment.snapshot.spec.blocks, 'web-ui': { services: { web: {} } } } } } }
    vi.spyOn(client, 'deployment').mockResolvedValue(inspected); vi.spyOn(client, 'profiles').mockResolvedValue({ apiVersion: 'v1', profiles: [{ ...trustedProfile, name: 'web-ui', deployment: 'other-deployment' }], sourceErrors: [] })
    render(<App client={client} />); await screen.findByRole('heading', { name: 'comparison', level: 1 }); await user.click(screen.getByRole('button', { name: 'Inspect ui-feature' }))
    const inspector = screen.getByRole('complementary', { name: 'Inspector' }); expect(await within(inspector).findByText('web-ui · not listed in the profile library')).toBeInTheDocument()
  })

  it('shows an honest empty state when an instance has no attributed operations', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test'); const operations = vi.spyOn(client, 'operations').mockResolvedValue({ apiVersion: 'v1', operations: [], nextCursor: null })
    render(<App client={client} />); await screen.findByRole('heading', { name: 'comparison', level: 1 }); await user.click(screen.getByRole('button', { name: 'Inspect ui-feature' }))
    const inspector = screen.getByRole('complementary', { name: 'Inspector' }); expect(await within(inspector).findByText('No instance-scoped operations recorded for ui-feature.')).toBeInTheDocument(); expect(operations).toHaveBeenCalledWith({ deployment: 'comparison', instance: 'ui-feature' })
  })

  it('uses the same single inspector when patch-bay selection changes the instance', async () => {
    const user = userEvent.setup(); render(<App client={new ApiClient('test')} />); await screen.findByRole('heading', { name: 'comparison', level: 1 }); await user.click(screen.getByRole('button', { name: 'Inspect ui-feature' }))
    const inspector = screen.getByRole('complementary', { name: 'Inspector' }); expect(within(inspector).getByRole('heading', { name: 'ui-feature' })).toBeInTheDocument()
    const providers = screen.getByRole('heading', { name: 'Backends & providers' }).parentElement!; await user.click(within(providers).getByRole('button', { name: /backend-a/ }))
    expect(within(inspector).getByRole('heading', { name: 'backend-a' })).toBeInTheDocument(); expect(screen.getAllByRole('complementary', { name: 'Inspector' })).toHaveLength(1); expect(screen.queryByLabelText('Instance inspector')).not.toBeInTheDocument()
  })

  it('renders authored desired connections including unbound consumers while stopped', async () => {
    const stopped = { ...deployment, resources: [], customDomains: [], reconciliation: { deployment: 'comparison', diagnostics: [{ code: 'observed_resources_missing', path: 'observed.resources', message: 'no labeled Docker resources were observed' }] } }; const client = new ApiClient('test')
    vi.spyOn(client, 'deployment').mockResolvedValue(stopped); vi.spyOn(client, 'definition').mockResolvedValue(authoredDefinition); vi.spyOn(client, 'validateDeployment').mockResolvedValue({ apiVersion: 'v1', name: 'comparison', valid: true, diagnostics: [], preview: { definition: { spec: authoredSpec } } })
    render(<App client={client} />)
    expect(await screen.findByText('Deployment is stopped or cleaned up')).toBeInTheDocument(); expect(screen.getByRole('button', { name: 'Run Up' })).toBeInTheDocument(); expect(screen.getAllByText('not running')).toHaveLength(6)
    expect(await screen.findByRole('heading', { name: 'Desired connections (authored state)' })).toBeInTheDocument()
    expect(screen.getByText('Desired/authored state from the deployment definition, not observed/runtime state. Changes take effect on the next Up.')).toBeInTheDocument()
    expect(screen.getByRole('rowheader', { name: 'ui-feature' })).toBeInTheDocument()
    expect(screen.getByRole('rowheader', { name: 'ui-unbound' })).toBeInTheDocument()
    expect(screen.getByLabelText('Desired provider group for ui-unbound')).toHaveValue('')
    expect(screen.getByText('Unbound — no desired provider')).toBeInTheDocument()
    expect(screen.queryByRole('img', { name: 'Route cables' })).not.toBeInTheDocument(); expect(screen.getByText('Unavailable while stopped')).toBeInTheDocument()
  })

  it('saves an offline desired connection edit through validation with the expected hash', async () => {
    const user = userEvent.setup(); const stopped = { ...deployment, resources: [], customDomains: [], reconciliation: { deployment: 'comparison', diagnostics: [{ code: 'observed_resources_missing', path: 'observed.resources', message: 'no labeled Docker resources were observed' }] } }; const client = new ApiClient('test')
    vi.spyOn(client, 'deployment').mockResolvedValue(stopped); vi.spyOn(client, 'definition').mockResolvedValue(authoredDefinition); vi.spyOn(client, 'validateDeployment').mockResolvedValue({ apiVersion: 'v1', name: 'comparison', valid: true, diagnostics: [], preview: { definition: { spec: authoredSpec } } }); const update = vi.spyOn(client, 'updateDefinitionValidated').mockImplementation(async (_name, yaml) => ({ ...authoredDefinition, hash: 'hash-two', yaml }))
    render(<App client={client} />); const select = await screen.findByLabelText('Desired provider group for ui-unbound'); await user.selectOptions(select, 'base'); await user.click(screen.getByRole('button', { name: 'Save desired connections' }))
    await waitFor(() => expect(update).toHaveBeenCalledWith('comparison', expect.stringContaining('"ui-unbound": "base"'), 'hash-one'))
  })

  it('deregisters an unmanaged source without a dirty-state guard', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test'); const deregister = vi.spyOn(client, 'deregisterSource').mockResolvedValue(); vi.spyOn(client, 'sources').mockResolvedValue([unmanagedSource])
    render(<App client={client} />)
    await user.click(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'sources' }))
    const card = (await screen.findByRole('heading', { name: 'shared-app' })).closest('article')!; await user.click(within(card).getByRole('button', { name: 'Remove' }))
    const dialog = screen.getByRole('dialog'); expect(within(dialog).getByText('This forgets only the registration. Files on disk are untouched.')).toBeInTheDocument(); expect(within(dialog).queryByText(/Dirty worktree|Second step/)).not.toBeInTheDocument()
    await user.click(within(dialog).getByRole('button', { name: 'Confirm removal' }))
    await waitFor(() => expect(deregister).toHaveBeenCalledWith('shared-app'))
  })

  it('removes a managed worktree only after the dirty-state guard', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test'); const removeWorktree = vi.spyOn(client, 'removeWorktree').mockResolvedValue({ staged: 1, unstaged: 2, untracked: 3 })
    render(<App client={client} />)
    await user.click(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'sources' }))
    await user.click(await screen.findByRole('button', { name: 'Remove' }))
    const dialog = screen.getByRole('dialog'); expect(within(dialog).getByText('This deletes the managed worktree directory from disk.')).toBeInTheDocument(); expect(within(dialog).getByText(/1 staged, 2 unstaged, 3 untracked/)).toBeInTheDocument()
    await user.click(within(dialog).getByRole('button', { name: 'Review dirty removal' }))
    expect(removeWorktree).not.toHaveBeenCalled(); expect(screen.getByText(/Second step/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Confirm removal' }))
    await waitFor(() => expect(removeWorktree).toHaveBeenCalledWith('feature-ui', true))
  })

  it('reviews a source manifest before import and validates the expanded profile', async () => {
    const user = userEvent.setup(); const fetchMock = vi.mocked(fetch)
    render(<App client={new ApiClient('test')} />)
    await user.click(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'profiles' }))
    expect(await screen.findByText('Profile editing is not available')).toBeInTheDocument()
    expect(screen.getByText('not imported')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Review manifest to import' }))
    const dialog = await screen.findByRole('dialog', { name: 'Review and trust import' })
    expect(within(dialog).getByText(/version: 1/)).toBeInTheDocument()
    expect(fetchMock.mock.calls.some(([url]) => String(url).includes('/profiles/api/manifest?source=feature-ui'))).toBe(true)
    await user.click(within(dialog).getByRole('button', { name: 'Import reviewed manifest' }))
    await waitFor(() => expect(fetchMock.mock.calls.some(([url, init]) => String(url).endsWith('/profiles/api/import') && init?.body === JSON.stringify({ source: 'feature-ui', reviewedManifestHash: 'review-one' }))).toBe(true))

    await user.click(screen.getByRole('button', { name: 'Inspect' }))
    expect(await screen.findByText(/"image": "busybox"/)).toBeInTheDocument()
    await user.selectOptions(screen.getByLabelText('Validate against checkout'), 'feature-ui')
    await user.click(screen.getByRole('button', { name: 'Validate expansion' }))
    expect(await screen.findByText('Validation passed')).toBeInTheDocument()
    expect(screen.getByText('comparison--profile-validation-preview--web')).toBeInTheDocument()
  })

  it('shows changed imported profiles as requiring re-review and supports removal', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test'); const changed = { ...sourceProfile, origin: { kind: 'imported-from-source' as const, source: 'feature-ui', commit: 'old' }, trust: 'changed' as const }
    vi.spyOn(client, 'profiles').mockResolvedValue({ apiVersion: 'v1', profiles: [changed], sourceErrors: [] })
    const remove = vi.spyOn(client, 'removeProfile').mockResolvedValue(); vi.spyOn(window, 'confirm').mockReturnValue(true)
    render(<App client={client} />)
    await user.click(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'profiles' }))
    expect(await screen.findByText('changed — review again')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Review changed manifest' })).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Remove imported' }))
    await waitFor(() => expect(remove).toHaveBeenCalledWith('api'))
  })

  it('lists only structured authoring, previews exact commands, and acknowledges shell execution', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test')
    const actions: RunActionsResponse = {
      apiVersion: 'v1', shellNoticeAcknowledged: false,
      actions: [
        { name: 'dev plan', description: 'Plan development', type: 'structured', command: 'plan', overlays: ['overlays/dev.yaml'], variation: 'fast', set: ['LOG_LEVEL=debug'] },
        { name: 'shell status', description: 'Legacy terminal command', type: 'shell', command: 'git status --short' },
      ],
    }
    const structuredPreview: RunActionPreview = { apiVersion: 'v1', name: 'dev plan', description: 'Plan development', target: { kind: 'deployment', name: 'comparison', bundle: '.switchyard/generated/comparison/resolved-deployment.yaml' }, execution: { type: 'structured', argv: ['plan', '.switchyard/generated/comparison/resolved-deployment.yaml', '--with', 'overlays/dev.yaml', '--variation', 'fast', '--set', 'LOG_LEVEL=debug'] }, shellNoticeAcknowledged: false, shellAcknowledgementRequired: false, previewHash: 'structured-preview' }
    const shellPreview: RunActionPreview = { apiVersion: 'v1', name: 'shell status', description: 'Legacy terminal command', target: { kind: 'project-shell-context', root: '/project' }, execution: { type: 'shell', command: 'git status --short' }, shellNoticeAcknowledged: false, shellAcknowledgementRequired: true, previewHash: 'shell-preview' }
    const operation: Operation = { apiVersion: 'v1', id: 'op-run', deployment: 'comparison', instance: null, kind: 'run-action', destructive: false, status: 'running', startedAt: 10, finishedAt: null, error: null, result: null }
    vi.spyOn(client, 'runActions').mockResolvedValue(actions)
    vi.spyOn(client, 'previewRunAction').mockImplementation(async (name) => name === 'dev plan' ? structuredPreview : shellPreview)
    const execute = vi.spyOn(client, 'executeRunAction').mockResolvedValue(operation)
    vi.spyOn(client, 'pollOperation').mockResolvedValue({ ...operation, status: 'succeeded', finishedAt: 11, result: { exitCode: 0, stdout: '', stderr: '' } })
    render(<App client={client} />)

    await user.click(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'run actions' }))
    expect(await screen.findByText('Shell action authoring is unavailable in the browser')).toBeInTheDocument()
    expect(screen.getByText(/Create and edit shell actions through the CLI or TUI/)).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Create structured action' })).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: 'Create shell action' })).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Preview and run' }))
    const structuredDialog = await screen.findByRole('dialog', { name: 'Confirm run action · dev plan' })
    expect(within(structuredDialog).getByText('Argument vector')).toBeInTheDocument()
    expect(within(structuredDialog).getByText('plan .switchyard/generated/comparison/resolved-deployment.yaml --with overlays/dev.yaml --variation fast --set LOG_LEVEL=debug')).toBeInTheDocument()
    await user.click(within(structuredDialog).getByRole('button', { name: 'Confirm and run' }))
    await waitFor(() => expect(execute).toHaveBeenCalledWith('dev plan', structuredPreview, false))

    await user.click(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'run actions' }))
    await user.click(await screen.findByRole('button', { name: 'Preview and run shell action' }))
    const shellDialog = await screen.findByRole('dialog', { name: 'Confirm run action · shell status' })
    expect(within(shellDialog).getByText('Shell command')).toBeInTheDocument()
    expect(within(shellDialog).getByText('git status --short')).toBeInTheDocument()
    expect(within(shellDialog).getByText(/Shell execution acknowledgement/)).toBeInTheDocument()
    await user.click(within(shellDialog).getByRole('button', { name: 'Acknowledge and run' }))
    await waitFor(() => expect(execute).toHaveBeenCalledWith('shell status', shellPreview, true))
  })

  it('loads durable operations and keeps cancellation for active records', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test')
    vi.spyOn(client, 'operations').mockResolvedValue({ apiVersion: 'v1', operations: [{ apiVersion: 'v1', id: 'op-cli-running', deployment: 'comparison', instance: null, kind: 'down', destructive: true, status: 'running', startedAt: 5, finishedAt: null, error: null, result: null }], nextCursor: null })
    const cancel = vi.spyOn(client, 'cancel').mockResolvedValue({ apiVersion: 'v1', id: 'op-cli-running', deployment: 'comparison', instance: null, kind: 'down', destructive: true, status: 'cancelled', startedAt: 5, finishedAt: 6, error: null, result: null })
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    render(<App client={client} />)
    await user.click(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'operations' }))
    expect(await screen.findByText('op-cli-running')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Cancel' }))
    await waitFor(() => expect(cancel).toHaveBeenCalledWith('op-cli-running'))
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Cancel' })).not.toBeInTheDocument())
  })

  it('renders live SSE fixtures in the operation drawer', async () => {
    const user = userEvent.setup()
    render(<App client={new ApiClient('test')} />)
    await screen.findByRole('heading', { name: 'comparison', level: 1 })
    await user.click(screen.getByRole('button', { name: 'Validate' }))
    const event: OperationEvent = { id: 1, operationId: 'op-new', kind: 'build', timestamp: 10, data: { line: 'Build completed: ui-feature' } }
    MockEventSource.instances[0].emit(event)
    expect(await screen.findByText(/Build completed: ui-feature/)).toBeInTheDocument()
    expect((await screen.findAllByText('succeeded')).length).toBeGreaterThan(0)
  })

  it('narrows the event drawer with a case-insensitive free-text filter', async () => {
    const user = userEvent.setup()
    render(<App client={new ApiClient('test')} />)
    await screen.findByRole('heading', { name: 'comparison', level: 1 }); await user.click(screen.getByRole('button', { name: 'Validate' }))
    MockEventSource.instances[0].emit({ id: 1, operationId: 'op-new', kind: 'log', timestamp: 10, data: { line: 'api-service ready on ui-feature' } })
    MockEventSource.instances[0].emit({ id: 2, operationId: 'op-new', kind: 'log', timestamp: 11, data: { line: 'worker queue drained' } })
    expect(await screen.findByText(/api-service ready/)).toBeInTheDocument(); expect(screen.getByText(/worker queue drained/)).toBeInTheDocument()
    await user.type(screen.getByLabelText('Filter events and logs'), 'API-SERVICE')
    expect(screen.getByText(/api-service ready/)).toBeInTheDocument(); expect(screen.queryByText(/worker queue drained/)).not.toBeInTheDocument()
  })

  it('composes the deployment select and free-text event filters', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test'); const summaries = [{ name: 'comparison', definitionHash: 'definition123', resourceHash: 'resource123', appliedAt: 1, lastOperation: null, customDomains: [], bindings: {} }, { name: 'staging', definitionHash: 'definition456', resourceHash: 'resource456', appliedAt: 2, lastOperation: null, customDomains: [], bindings: {} }]
    vi.spyOn(client, 'deployments').mockResolvedValue({ apiVersion: 'v1', deployments: summaries }); vi.spyOn(client, 'deployment').mockImplementation(async (name) => ({ ...deployment, deployment: name, reconciliation: { ...deployment.reconciliation, deployment: name } })); vi.spyOn(client, 'routes').mockImplementation(async (name) => ({ ...routeState, deployment: name }))
    vi.spyOn(client, 'command').mockImplementation(async (_kind, bundle) => ({ apiVersion: 'v1', id: bundle.includes('staging') ? 'op-staging' : 'op-comparison', deployment: bundle.includes('staging') ? 'staging' : 'comparison', instance: null, kind: 'validate', destructive: false, status: 'running', startedAt: 10, finishedAt: null, error: null, result: null })); vi.spyOn(client, 'pollOperation').mockImplementation(async (id) => ({ apiVersion: 'v1', id, deployment: id === 'op-staging' ? 'staging' : 'comparison', instance: null, kind: 'validate', destructive: false, status: 'succeeded', startedAt: 10, finishedAt: 11, error: null, result: { exitCode: 0, stdout: '', stderr: '' } }))
    render(<App client={client} />); await screen.findByRole('heading', { name: 'comparison', level: 1 }); await user.click(screen.getByRole('button', { name: 'Validate' })); MockEventSource.instances[0].emit({ id: 1, operationId: 'op-comparison', kind: 'log', timestamp: 10, data: { line: 'shared service comparison output' } })
    await user.click(screen.getByRole('button', { name: /^staging/ })); await screen.findByRole('heading', { name: 'staging', level: 1 }); await user.click(screen.getByRole('button', { name: 'Validate' })); MockEventSource.instances[1].emit({ id: 2, operationId: 'op-staging', kind: 'log', timestamp: 11, data: { line: 'shared service staging output' } })
    await user.selectOptions(screen.getByLabelText('Deployment'), 'staging'); await user.type(screen.getByLabelText('Filter events and logs'), 'shared service')
    expect(screen.getByText(/shared service staging output/)).toBeInTheDocument(); expect(screen.queryByText(/shared service comparison output/)).not.toBeInTheDocument()
    await user.clear(screen.getByLabelText('Filter events and logs')); await user.type(screen.getByLabelText('Filter events and logs'), 'comparison')
    expect(screen.queryByText(/shared service comparison output/)).not.toBeInTheDocument(); expect(screen.queryByText(/shared service staging output/)).not.toBeInTheDocument()
  })

  it('copies exactly the free-text-filtered event set as plain text', async () => {
    const user = userEvent.setup(); const writeText = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue()
    render(<App client={new ApiClient('test')} />)
    await screen.findByRole('heading', { name: 'comparison', level: 1 }); await user.click(screen.getByRole('button', { name: 'Validate' }))
    MockEventSource.instances[0].emit({ id: 1, operationId: 'op-new', kind: 'log', timestamp: 10, data: { line: 'include api-service output' } }); MockEventSource.instances[0].emit({ id: 2, operationId: 'op-new', kind: 'log', timestamp: 11, data: { line: 'exclude worker output' } })
    await user.type(screen.getByLabelText('Filter events and logs'), 'api-service'); await user.click(screen.getByRole('button', { name: 'Copy plain text' }))
    expect(writeText).toHaveBeenCalledWith('include api-service output')
  })

  it('renders destructive timeline markers only from the operation field', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test'); vi.spyOn(client, 'operations').mockResolvedValue({ apiVersion: 'v1', operations: [{ apiVersion: 'v1', id: 'op-destructive', deployment: 'comparison', instance: null, kind: 'cleanup', destructive: true, status: 'succeeded', startedAt: 5, finishedAt: 6, error: null, result: null }, { apiVersion: 'v1', id: 'op-safe', deployment: 'comparison', instance: null, kind: 'down', destructive: false, status: 'succeeded', startedAt: 3, finishedAt: 4, error: null, result: null }], nextCursor: null })
    render(<App client={client} />); await user.click(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'operations' }))
    const destructiveRow = (await screen.findByText('op-destructive')).closest('li')!; const safeRow = screen.getByText('op-safe').closest('li')!
    expect(within(destructiveRow).getByText('Destructive')).toBeInTheDocument(); expect(within(safeRow).queryByText('Destructive')).not.toBeInTheDocument()
  })

  it('filters the durable timeline by free text across fields and captured output', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test'); vi.spyOn(client, 'operations').mockResolvedValue({ apiVersion: 'v1', operations: [{ apiVersion: 'v1', id: 'op-cleanup', deployment: 'comparison', instance: null, kind: 'cleanup', destructive: true, status: 'succeeded', startedAt: 5, finishedAt: 6, error: null, result: null }, { apiVersion: 'v1', id: 'op-apply', deployment: 'staging', instance: null, kind: 'apply', destructive: false, status: 'failed', startedAt: 3, finishedAt: 4, error: null, result: { exitCode: 1, stdout: '', stderr: 'checkout-ui refused to start' } }], nextCursor: null })
    render(<App client={client} />); await user.click(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'operations' }))
    expect(await screen.findByText('op-cleanup')).toBeInTheDocument()
    const filter = screen.getByRole('textbox', { name: /Filter operations/ })
    await user.type(filter, 'STAGING')
    expect(screen.queryByText('op-cleanup')).not.toBeInTheDocument(); expect(screen.getByText('op-apply')).toBeInTheDocument()
    await user.clear(filter); await user.type(filter, 'checkout-ui refused')
    expect(screen.getByText('op-apply')).toBeInTheDocument(); expect(screen.queryByText('op-cleanup')).not.toBeInTheDocument()
    await user.clear(filter); await user.type(filter, 'nothing-matches-this')
    expect(screen.getByText('No operations match this filter.')).toBeInTheDocument()
  })

  it('lands on Home when the project has no deployments', async () => {
    const client = new ApiClient('test'); vi.spyOn(client, 'deployments').mockResolvedValue({ apiVersion: 'v1', deployments: [] })
    render(<App client={client} />)
    expect(await screen.findByRole('heading', { name: 'payments-lab', level: 1 })).toBeInTheDocument()
    expect(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'home' })).toHaveAttribute('aria-current', 'page')
    expect(screen.getByRole('heading', { name: 'Setup progress' })).toBeInTheDocument()
  })

  it('keeps Deployments as the landing view when deployments already exist', async () => {
    render(<App client={new ApiClient('test')} />)
    expect(await screen.findByRole('heading', { name: 'comparison', level: 1 })).toBeInTheDocument()
    expect(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'deployments' })).toHaveAttribute('aria-current', 'page')
    expect(screen.queryByRole('heading', { name: 'Setup progress' })).not.toBeInTheDocument()
  })

  it('moves every setup checklist step from incomplete to complete from API signals', async () => {
    const empty = new ApiClient('test'); vi.spyOn(empty, 'deployments').mockResolvedValue({ apiVersion: 'v1', deployments: [] }); vi.spyOn(empty, 'sources').mockResolvedValue([]); vi.spyOn(empty, 'profiles').mockResolvedValue({ apiVersion: 'v1', profiles: [], sourceErrors: [] }); vi.spyOn(empty, 'devices').mockResolvedValue([localDevice]); vi.spyOn(empty, 'operations').mockResolvedValue({ apiVersion: 'v1', operations: [], nextCursor: null })
    render(<App client={empty} />); await screen.findByRole('heading', { name: 'Setup progress' })
    await waitFor(() => expect(screen.getAllByText('Not complete')).toHaveLength(5))
    for (const label of ['Source registered', 'Profile selected', 'Instance created', 'Startup complete', 'Connection bound']) expect(within(screen.getByText(label).closest('li')!).getByText('Not complete')).toBeInTheDocument()
    cleanup()

    const ready = new ApiClient('test'); const summary = { name: 'comparison', definitionHash: 'definition123', resourceHash: 'resource123', appliedAt: 1, lastOperation: null, customDomains: [], bindings: { ui: 'base' } }; const complete = { ...deployment, appliedAt: 1, snapshot: { spec: { instances: [{ name: 'ui', block: 'web' }, { name: 'backend', block: 'api' }], blocks: { web: { services: { app: { consumes: { api: { protocol: 'tcp' } } } } }, api: { services: { app: { provides: { api: {} } } } } }, groups: { base: { instances: ['backend'] } }, bindings: { ui: 'base' } } } }
    vi.spyOn(ready, 'deployments').mockResolvedValue({ apiVersion: 'v1', deployments: [summary] }); vi.spyOn(ready, 'deployment').mockResolvedValue(complete); vi.spyOn(ready, 'definition').mockResolvedValue(authoredDefinition); vi.spyOn(ready, 'validateDeployment').mockResolvedValue({ apiVersion: 'v1', name: 'comparison', valid: true, diagnostics: [], preview: { definition: complete.snapshot } }); vi.spyOn(ready, 'sources').mockResolvedValue([source]); vi.spyOn(ready, 'profiles').mockResolvedValue({ apiVersion: 'v1', profiles: [trustedProfile], sourceErrors: [] }); vi.spyOn(ready, 'devices').mockResolvedValue([localDevice]); vi.spyOn(ready, 'operations').mockResolvedValue({ apiVersion: 'v1', operations: [], nextCursor: null })
    render(<App client={ready} />); await screen.findByRole('heading', { name: 'comparison', level: 1 }); await userEvent.click(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'home' }))
    await waitFor(() => expect(screen.getAllByText('Complete')).toHaveLength(5))
  })

  it('navigates the next recommended Create instance action into the builder', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test'); vi.spyOn(client, 'deployments').mockResolvedValue({ apiVersion: 'v1', deployments: [] }); vi.spyOn(client, 'sources').mockResolvedValue([source]); vi.spyOn(client, 'profiles').mockResolvedValue({ apiVersion: 'v1', profiles: [trustedProfile], sourceErrors: [] })
    render(<App client={client} />)
    await user.click(await screen.findByRole('button', { name: 'Create an instance' }))
    expect(await screen.findByRole('heading', { name: 'New deployment', level: 1 })).toBeInTheDocument()
  })

  it('aggregates source, profile, and operation problems on Home', async () => {
    const client = new ApiClient('test'); const failed: Operation = { apiVersion: 'v1', id: 'op-failed', deployment: 'comparison', instance: null, kind: 'apply', destructive: false, status: 'failed', startedAt: 5, finishedAt: 6, error: { code: 'apply_failed', message: 'container startup failed' }, result: null }
    vi.spyOn(client, 'deployments').mockResolvedValue({ apiVersion: 'v1', deployments: [] }); vi.spyOn(client, 'sources').mockResolvedValue([{ ...source, inspection: { ...source.inspection, unknownCode: 'git_unavailable' } }]); vi.spyOn(client, 'profiles').mockResolvedValue({ apiVersion: 'v1', profiles: [], sourceErrors: [{ source: 'feature-ui', message: 'manifest is invalid' }] }); vi.spyOn(client, 'devices').mockResolvedValue([localDevice]); vi.spyOn(client, 'operations').mockResolvedValue({ apiVersion: 'v1', operations: [failed], nextCursor: null })
    render(<App client={client} />); await screen.findByRole('heading', { name: 'Project-wide problems' })
    expect(await screen.findByText('feature-ui: inspection unavailable (git_unavailable).')).toBeInTheDocument()
    expect(screen.getByText('feature-ui: manifest is invalid')).toBeInTheDocument()
    expect(screen.getByText('comparison apply: container startup failed.')).toBeInTheDocument()
  })

  it('switches shell views with keyboard arrow navigation', async () => {
    const user = userEvent.setup()
    render(<App client={new ApiClient('test')} />)
    const deployments = within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'deployments' })
    deployments.focus()
    await user.keyboard('{ArrowRight}')
    expect(await screen.findByRole('heading', { name: 'Sources', level: 1 })).toBeInTheDocument()
  })

  it('renders reachability and eligibility separately and refreshes both after a check', async () => {
    const user = userEvent.setup(); render(<App client={new ApiClient('test')} />)
    await user.click(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'devices' }))
    expect(await screen.findByRole('cell', { name: 'dev@host.test:22' })).toBeInTheDocument()
    expect(screen.getByRole('columnheader', { name: 'Reachability' })).toBeInTheDocument()
    expect(screen.getByRole('columnheader', { name: 'Eligibility' })).toBeInTheDocument()
    expect(screen.getAllByText('unchecked')).toHaveLength(2)
    expect(screen.getByText('ineligible')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Check eligibility' }))
    expect((await screen.findAllByText('eligible')).length).toBeGreaterThan(0)
    expect(screen.getAllByText('reachable').length).toBeGreaterThan(0)
  })

  it('shows placements and blocks removal of an occupied device', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test'); const removeDevice = vi.spyOn(client, 'removeDevice').mockResolvedValue()
    render(<App client={client} />)
    await user.click(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'devices' }))
    const row = (await screen.findByRole('cell', { name: 'dev@host.test:22' })).closest('tr')!
    await user.click(within(row).getByRole('button', { name: 'Remove' }))
    const dialog = screen.getByRole('dialog')
    expect(within(dialog).getByText('comparison / ui-feature')).toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: 'Confirm removal' })).toBeDisabled()
    expect(removeDevice).not.toHaveBeenCalled()
  })

  it('shows inline add-device validation and submits a valid device', async () => {
    const user = userEvent.setup(); const fetchMock = vi.mocked(fetch); render(<App client={new ApiClient('test')} />)
    await user.click(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'devices' }))
    await screen.findByRole('heading', { name: 'Devices' })
    await user.clear(screen.getByLabelText('Port')); await user.type(screen.getByLabelText('Port'), '70000'); await user.click(screen.getByRole('button', { name: 'Add device' }))
    expect(screen.getByText('Name is required.')).toBeInTheDocument(); expect(screen.getByText('Port must be between 1 and 65535.')).toBeInTheDocument()
    await user.type(screen.getByLabelText('Name'), 'runner'); await user.type(screen.getByLabelText('User'), 'dev'); await user.type(screen.getByLabelText('Host'), 'runner.test'); await user.clear(screen.getByLabelText('Port')); await user.type(screen.getByLabelText('Port'), '2222'); await user.click(screen.getByRole('button', { name: 'Add device' }))
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith('/api/v1/devices', expect.objectContaining({ method: 'POST', body: JSON.stringify({ name: 'runner', user: 'dev', host: 'runner.test', port: 2222 }) })))
  })

  it('renders the observed runtime patch bay while running and performs a keyboard-only complete binding switch', async () => {
    const user = userEvent.setup(); const fetchMock = vi.mocked(fetch)
    render(<App client={new ApiClient('test')} />); await screen.findByRole('heading', { name: 'comparison', level: 1 })
    expect(screen.getByRole('heading', { name: 'Observed runtime patch bay' })).toBeInTheDocument(); expect(screen.getByText('Observed/runtime state from the applied snapshot.')).toBeInTheDocument(); expect(screen.getByRole('img', { name: 'Route cables' }).querySelectorAll('path[data-slot]')).toHaveLength(3)
    const lane = screen.getByRole('heading', { name: 'UI consumers' }).parentElement!; await user.click(within(lane).getByRole('button', { name: /ui-feature/ }))
    const select = screen.getByLabelText('Provider group for ui-feature'); select.focus(); await user.selectOptions(select, 'base')
    const dialog = screen.getByRole('dialog', { name: 'Preview complete route replacement' }); expect(within(dialog).getByText(/Snapshot v4/)).toBeInTheDocument(); expect(within(dialog).getAllByRole('row')).toHaveLength(4); expect(within(dialog).getByText('backend-b')).toBeInTheDocument()
    await user.click(within(dialog).getByRole('button', { name: 'Apply complete change' }))
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith('/api/v1/commands/bind', expect.objectContaining({ body: JSON.stringify({ bundle: '.switchyard/generated/comparison/resolved-deployment.yaml', consumer: 'ui-feature', group: 'base', transition: { strategy: 'close' } }) })))
    const result = await screen.findByRole('dialog', { name: 'Connection switch result' }); expect(within(result).getByText('Atomic binding operation succeeded.')).toBeInTheDocument(); expect(within(result).getByText(/Exit code 0\. applied v5/)).toBeInTheDocument(); expect(within(result).getByText(/desired v5; observed v4; status pending; transition draining; error none; rollback recorded at v4/)).toBeInTheDocument()
  })

  it('shows a failed post-switch report with durable error and rollback observations', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test'); const failedRoutes: RouteState = { ...routeState, bindings: [{ ...routeState.bindings[0], status: 'failed', transition: { state: 'rolled_back' }, lastErrorCode: 'provider_unhealthy' }] }
    vi.spyOn(client, 'pollOperation').mockResolvedValue({ apiVersion: 'v1', id: 'op-bind', deployment: 'comparison', instance: 'ui-feature', kind: 'bind', destructive: false, status: 'failed', startedAt: 10, finishedAt: 11, error: { code: 'route_apply_failed', message: 'provider health check rejected the candidate' }, result: { exitCode: 1, stdout: '', stderr: 'provider unhealthy' } }); vi.spyOn(client, 'routes').mockResolvedValue(failedRoutes)
    render(<App client={client} />); await screen.findByRole('heading', { name: 'comparison', level: 1 }); const lane = screen.getByRole('heading', { name: 'UI consumers' }).parentElement!; await user.click(within(lane).getByRole('button', { name: /ui-feature/ })); await user.selectOptions(screen.getByLabelText('Provider group for ui-feature'), 'base'); await user.click(within(screen.getByRole('dialog', { name: 'Preview complete route replacement' })).getByRole('button', { name: 'Apply complete change' }))
    const result = await screen.findByRole('dialog', { name: 'Connection switch result' }); expect(within(result).getByText('Atomic binding operation failed.')).toBeInTheDocument(); expect(within(result).getByText('route_apply_failed: provider health check rejected the candidate')).toBeInTheDocument(); expect(within(result).getByText(/status failed; transition rolled_back; error provider_unhealthy; rollback recorded at v4/)).toBeInTheDocument()
  })

  it('lists an authored unbound consumer and binds it through the complete change preview', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test'); const fetchMock = vi.mocked(fetch); const spec = deployment.snapshot.spec
    vi.spyOn(client, 'deployment').mockResolvedValue({ ...deployment, snapshot: { spec: { ...spec, instances: [...spec.instances, { name: 'ui-unbound', block: 'web-ui' }], blocks: { ...deployment.snapshot.spec.blocks, 'web-ui': { services: { web: { consumes: { java: { protocol: 'tcp' }, python: { protocol: 'grpc' }, database: { protocol: 'tcp' } } } } } } } } })
    render(<App client={client} />); await screen.findByRole('heading', { name: 'comparison', level: 1 })
    const lane = screen.getByRole('heading', { name: 'UI consumers' }).parentElement!; const consumer = within(lane).getByRole('button', { name: /ui-unbound.*unbound/ }); expect(consumer).toBeInTheDocument(); expect(screen.getAllByRole('cell', { name: 'Unbound — no current provider' })).toHaveLength(3); await user.click(consumer)
    expect(screen.getByText(/Currently unbound/)).toBeInTheDocument(); const select = screen.getByLabelText('Provider group for ui-unbound'); expect(within(select).getByRole('option', { name: 'Unbound — choose a provider group' })).toBeInTheDocument(); await user.selectOptions(select, 'base')
    const dialog = screen.getByRole('dialog', { name: 'Preview complete route replacement' }); expect(within(dialog).getByText(/There is no current provider group/)).toBeInTheDocument(); expect(within(dialog).getAllByText('There is no current provider')).toHaveLength(3); expect(within(dialog).getByText('backend-b')).toBeInTheDocument()
    await user.click(within(dialog).getByRole('button', { name: 'Apply complete change' }))
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith('/api/v1/commands/bind', expect.objectContaining({ body: JSON.stringify({ bundle: '.switchyard/generated/comparison/resolved-deployment.yaml', consumer: 'ui-unbound', group: 'base', transition: { strategy: 'close' } }) })))
  })

  it('adds an instance to an existing deployment with checkout-filtered profiles, eligibility gating, SchemaForm parameters, and expansion preview', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test'); const update = vi.spyOn(client, 'updateDefinition').mockResolvedValue({ apiVersion: 'v1', name: 'comparison', path: '/project/deployments/comparison.yaml', hash: 'hash-two', yaml: 'guided-draft' })
    vi.spyOn(client, 'profiles').mockResolvedValue({ apiVersion: 'v1', profiles: [trustedProfile, sourceProfile], sourceErrors: [] })
    vi.spyOn(client, 'profile').mockResolvedValue({ ...trustedProfile, definition: { parameters: { LOG_LEVEL: { required: true } }, services: { web: { publish: [8080], volumes: [{ name: 'cache', target: '/cache', readOnly: true }] } } } })
    vi.spyOn(client, 'validateProfile').mockImplementation(async (_profile, checkout, input = {}) => ({ apiVersion: 'v1', name: trustedProfile.name, deployment: 'comparison', checkout, valid: true, expandedServices: input.instanceName ? [`comparison--${input.instanceName}--web`] : ['comparison--profile-validation-preview--web'], services: [{ name: 'web', ports: [8080], volumes: [{ name: 'cache', target: '/cache', readOnly: true }] }], diagnostics: [], error: null, draft: input.instanceName ? 'guided-draft' : 'filter-draft' }))
    render(<App client={client} />); await screen.findByRole('heading', { name: 'comparison', level: 1 }); await user.click(screen.getByRole('button', { name: 'Add instance' }))
    expect(await screen.findByRole('heading', { name: 'Add instance to comparison' })).toBeInTheDocument(); await user.selectOptions(screen.getByLabelText(/^Checkout \/ worktree/), 'feature-ui')
    const profileSelect = await screen.findByLabelText('Trusted startup profile'); await waitFor(() => expect(within(profileSelect).getByRole('option', { name: 'worker-profile' })).toBeInTheDocument()); expect(within(profileSelect).queryByRole('option', { name: 'api' })).not.toBeInTheDocument(); expect(screen.getByText(/api.*not-imported|not-imported.*review\/import/i)).toBeInTheDocument()
    await user.selectOptions(profileSelect, 'comparison:project:project:worker-profile')
    const deviceSelect = screen.getByLabelText('Device'); const remote = within(deviceSelect).getByRole('option', { name: /build-host.*unavailable: unchecked/ }); expect(remote).toBeDisabled(); expect(screen.getByText((_content, element) => element?.tagName === 'LI' && element.textContent === 'build-host: unchecked')).toBeInTheDocument(); await user.selectOptions(deviceSelect, 'local'); const parameter = await screen.findByLabelText(/LOG_LEVEL \*/); expect(parameter).toBeRequired(); await user.type(screen.getByLabelText(/^Instance name/), 'worker'); await user.type(parameter, 'debug')
    expect(await screen.findByText('comparison--worker--web')).toBeInTheDocument(); expect(screen.getByText('8080')).toBeInTheDocument(); expect(screen.getByText(/cache → \/cache \(read-only\)/)).toBeInTheDocument(); await user.click(screen.getByRole('button', { name: 'Append instance' }))
    await waitFor(() => expect(update).toHaveBeenCalledWith('comparison', 'guided-draft', 'hash-one'))
  })

  it('places profile, device, and parameter diagnostics beside their fields', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test')
    vi.spyOn(client, 'profiles').mockResolvedValue({ apiVersion: 'v1', profiles: [trustedProfile], sourceErrors: [] }); vi.spyOn(client, 'profile').mockResolvedValue({ ...trustedProfile, definition: { parameters: { TOKEN: { required: true } }, services: { web: {} } } })
    vi.spyOn(client, 'validateProfile').mockImplementation(async (_profile, checkout, input = {}) => input.instanceName ? { apiVersion: 'v1', name: trustedProfile.name, deployment: 'comparison', checkout, valid: false, expandedServices: [], services: [], diagnostics: [{ code: 'bad_profile', path: 'spec.instances.6.block', message: 'Profile cannot expand here.' }, { code: 'bad_device', path: 'spec.instances.6.device', message: 'Remote device is unavailable.' }, { code: 'bad_parameter', path: 'spec.instances.6.parameters.TOKEN', message: 'Token is rejected.' }], error: null, draft: 'invalid-draft' } : { apiVersion: 'v1', name: trustedProfile.name, deployment: 'comparison', checkout, valid: true, expandedServices: [], services: [], diagnostics: [], error: null, draft: 'filter-draft' })
    render(<App client={client} />); await screen.findByRole('heading', { name: 'comparison', level: 1 }); await user.click(screen.getByRole('button', { name: 'Add instance' })); await user.selectOptions(screen.getByLabelText(/^Checkout \/ worktree/), 'feature-ui'); const profileSelect = await screen.findByLabelText('Trusted startup profile'); await waitFor(() => expect(within(profileSelect).getByRole('option', { name: 'worker-profile' })).toBeInTheDocument()); await user.selectOptions(profileSelect, 'comparison:project:project:worker-profile'); await user.selectOptions(screen.getByLabelText('Device'), 'local'); await screen.findByLabelText(/TOKEN \*/); await user.type(screen.getByLabelText(/^Instance name/), 'worker'); await user.type(screen.getByLabelText(/TOKEN \*/), 'secret')
    expect(await screen.findByText('Profile cannot expand here.')).toBeInTheDocument(); expect(profileSelect).toHaveAttribute('aria-invalid', 'true'); expect(screen.getByLabelText('Device')).toHaveAttribute('aria-invalid', 'true'); expect(screen.getByLabelText(/TOKEN \*/)).toHaveAttribute('aria-invalid', 'true'); expect(screen.getByText('Remote device is unavailable.')).toBeInTheDocument(); expect(screen.getByText('Token is rejected.')).toBeInTheDocument(); expect(screen.getByRole('button', { name: 'Append instance' })).toBeDisabled()
  })

  it('builder validates a schema-driven draft and saves it', async () => {
    const user = userEvent.setup(); const fetchMock = vi.mocked(fetch); render(<App client={new ApiClient('test')} />)
    await user.click(screen.getByRole('button', { name: /New deployment/ })); await user.type(screen.getByLabelText(/^Name/), 'demo'); await user.type(screen.getByLabelText(/^Instance name/), 'worker'); await user.type(screen.getByLabelText('Block name'), 'service'); await user.selectOptions(screen.getByLabelText('Source'), 'feature-ui')
    await user.click(screen.getByRole('button', { name: 'Validate draft' })); expect(await screen.findByText('Expanded services')).toBeInTheDocument(); await user.click(screen.getByRole('button', { name: 'Save deployment' }))
    await waitFor(() => expect(fetchMock.mock.calls.some(([url, init]) => String(url).endsWith('/deployments') && init?.method === 'POST' && !JSON.parse(String(init.body)).validateOnly)).toBe(true))
  })

  it('shows a domain YAML diff and validates before definition PUT', async () => {
    const user = userEvent.setup(); const fetchMock = vi.mocked(fetch); render(<App client={new ApiClient('test')} />); await screen.findByRole('heading', { name: 'comparison', level: 1 }); await user.click(screen.getByRole('button', { name: 'Load routing definition' })); const editor = await screen.findByLabelText('Deployment YAML'); await user.type(editor, '  hostRouter: {{}}\n'); expect(screen.getByRole('heading', { name: 'Full YAML diff' })).toBeInTheDocument(); await user.click(screen.getByRole('button', { name: 'Validate changes' })); await user.click(await screen.findByRole('button', { name: 'Apply definition edit' }));
    await waitFor(() => { const calls = fetchMock.mock.calls; const put = calls.findIndex(([url, init]) => String(url).endsWith('/definition') && init?.method === 'PUT'); const validation = calls.findIndex(([url, init]) => String(url).endsWith('/deployments') && init?.method === 'POST' && JSON.parse(String(init.body)).validateOnly); expect(validation).toBeGreaterThanOrEqual(0); expect(put).toBeGreaterThan(validation) })
  })

  it('shows planner warnings returned by deployment validation', async () => {
    const client = new ApiClient('test'); vi.spyOn(client, 'definition').mockResolvedValue(authoredDefinition); vi.spyOn(client, 'validateDeployment').mockResolvedValue({ apiVersion: 'v1', name: 'comparison', valid: true, diagnostics: [], warnings: [{ code: 'provider_collision', path: 'spec.bindings.backend-1', message: '`database` slot has two candidates; routing to db-main, the first listed' }], preview: { definition: deployment.snapshot } })
    render(<App client={client} />); await screen.findByRole('heading', { name: 'comparison', level: 1 })
    const warnings = await screen.findByRole('complementary', { name: 'Planner warnings' }); expect(within(warnings).getByText('provider_collision')).toBeInTheDocument(); expect(within(warnings).getByText('spec.bindings.backend-1')).toBeInTheDocument(); expect(within(warnings).getByText(/routing to db-main, the first listed/)).toBeInTheDocument()
  })
})
