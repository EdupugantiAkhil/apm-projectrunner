import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import App from './App'
import { ApiClient, type Operation, type OperationEvent, type RunActionPreview, type RunActionsResponse, type RouteState } from './api'

const deployment = {
  apiVersion: 'v1', deployment: 'comparison', definitionHash: 'definition123', resourceHash: 'resource123', appliedAt: 1,
  snapshot: { spec: { instances: [{ name: 'ui-feature', device: 'build-host' }, { name: 'backend-a' }, { name: 'backend-b' }, { name: 'python-a' }, { name: 'python-b' }, { name: 'shared-db' }], bindings: { 'ui-feature': 'feature' }, routes: { 'ui-feature': { java: 'backend-a', python: 'python-a', database: 'shared-db' } }, groups: { base: { providers: { java: 'backend-b', python: 'python-b', database: 'shared-db' } }, feature: { providers: { java: 'backend-a', python: 'python-a', database: 'shared-db' } } }, uiRoutes: { browser: { origin: 'https://ui.comparison.localhost', backend: 'backend-a', downstreamGroup: 'feature' } } } }, manifest: {},
  sourceIdentities: { 'ui-feature': { path: '/worktrees/ui-a', ref: 'feature/ui-redesign', commit: '35ad2abcdef', dirty: true } },
  reconciliation: { deployment: 'comparison', diagnostics: [] }, resources: [{ kind: 'container', id: 'one', name: 'comparison-ui-feature', labels: { 'dev.switchyard.instance': 'ui-feature' }, state: 'healthy', device: 'build-host' }],
  customDomains: ['ui.comparison.localhost'], bindings: { 'ui-feature': 'feature' },
}
const source = { source: { name: 'feature-ui', kind: 'managed', path: '/worktrees/ui-a' }, inspection: { identity: { path: '/worktrees/ui-a', ref: 'feature/ui-redesign', commit: '35ad2abcdef', dirty: true }, branch: 'feature/ui-redesign', changes: { staged: 1, unstaged: 2, untracked: 3 }, ahead: 2, behind: 0, unknownCode: null } }
const unmanagedSource = { source: { name: 'shared-app', kind: 'unmanaged' as const, path: '/code/shared-app' }, inspection: { identity: { path: '/code/shared-app', ref: 'main', commit: '123456789ab', dirty: true }, branch: 'main', changes: { staged: 4, unstaged: 5, untracked: 6 }, ahead: 0, behind: 0, unknownCode: null } }
const sourceProfile = { apiVersion: 'v1', name: 'api', deployment: 'comparison', origin: { kind: 'discovered-in-source' as const, source: 'feature-ui', commit: '35ad2abcdef' }, trust: 'not-imported' as const, shadowed: false, services: [{ name: 'web', adapterKind: 'container' as const }] }
const trustedProfile = { apiVersion: 'v1', name: 'worker-profile', deployment: 'comparison', origin: { kind: 'project' as const }, trust: 'trusted' as const, shadowed: false, services: [{ name: 'web', adapterKind: 'container' as const }] }
const localDevice = { name: 'local', kind: 'local', host: null, port: null, user: null, identityFile: null, createdAt: null, lastCheckedAt: null, lastCheckStatus: 'eligible', lastCheckDetail: null, reachability: 'reachable', eligibility: 'eligible', eligibilityReason: 'local execution is always eligible', placedInstances: [] }
const remoteDevice = { name: 'build-host', kind: 'ssh', host: 'host.test', port: 22, user: 'dev', identityFile: null, createdAt: 1, lastCheckedAt: null, lastCheckStatus: 'never', lastCheckDetail: null, reachability: 'unchecked', eligibility: 'ineligible', eligibilityReason: 'unchecked', placedInstances: [{ deployment: 'comparison', instance: 'ui-feature' }] }
const authoredSpec = { instances: [{ name: 'ui-feature', block: 'web-ui' }, { name: 'ui-unbound', block: 'web-ui' }, { name: 'backend-a' }, { name: 'backend-b' }, { name: 'python-a' }, { name: 'python-b' }, { name: 'shared-db' }], blocks: { 'web-ui': { services: { web: { consumes: { java: { protocol: 'tcp' }, python: { protocol: 'grpc' }, database: { protocol: 'tcp' } } } } } }, groups: { base: { providers: { java: 'backend-b', python: 'python-b', database: 'shared-db' } }, feature: { providers: { java: 'backend-a', python: 'python-a', database: 'shared-db' } } }, bindings: { 'ui-feature': 'feature' } }
const authoredYaml = 'apiVersion: switchyard.dev/v1alpha1\nkind: Deployment\nmetadata:\n  name: comparison\nspec:\n  instances: []\n  blocks: {}\n  groups: {}\n  bindings:\n    ui-feature: feature\n'
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
    if (url.endsWith('/operations')) return json({ apiVersion: 'v1', operations: [{ apiVersion: 'v1', id: 'op-cli', deployment: 'comparison', kind: 'cleanup', destructive: true, status: 'succeeded', startedAt: 5, finishedAt: 6, error: null, result: null }], nextCursor: null })
    if (url.endsWith('/deployments/comparison/definition') && (!init?.method || init.method === 'GET')) return json({ apiVersion: 'v1', name: 'comparison', path: '/project/deployments/comparison.yaml', hash: 'hash-one', yaml: 'metadata:\n  name: comparison\nspec:\n  uiRoutes: {}\n' })
    if (url.endsWith('/deployments/comparison/definition') && init?.method === 'PUT') return json({ apiVersion: 'v1', name: 'comparison', path: '/project/deployments/comparison.yaml', hash: 'hash-two', yaml: JSON.parse(String(init.body)).yaml })
    if (url.endsWith('/deployments') && init?.method === 'POST') { const body = JSON.parse(String(init.body)); if (body.validateOnly) return json({ apiVersion: 'v1', name: body.name, valid: true, diagnostics: [], preview: { expandedServiceCount: 1, routes: ['ui-feature'] } }); return json({ apiVersion: 'v1', name: body.name, path: `/project/deployments/${body.name}.yaml`, hash: 'new-hash', yaml: body.yaml }, 201) }
    if (url.includes('/worktrees/feature-ui')) return json({ staged: 1, unstaged: 2, untracked: 3 })
    if (url.endsWith('/commands/validate') && init?.method === 'POST') return json({ apiVersion: 'v1', id: 'op-new', deployment: 'comparison', kind: 'validate', status: 'running', startedAt: 10, finishedAt: null, error: null, result: null }, 202)
    if (url.endsWith('/commands/bind') && init?.method === 'POST') return json({ apiVersion: 'v1', id: 'op-bind', deployment: 'comparison', kind: 'bind', status: 'running', startedAt: 10, finishedAt: null, error: null, result: null }, 202)
    if (url.endsWith('/operations/op-new')) { operationReads += 1; return json({ apiVersion: 'v1', id: 'op-new', deployment: 'comparison', kind: 'validate', status: 'succeeded', startedAt: 10, finishedAt: 11, error: null, result: { exitCode: 0, stdout: 'valid', stderr: '' } }) }
    if (url.endsWith('/operations/op-bind')) return json({ apiVersion: 'v1', id: 'op-bind', deployment: 'comparison', kind: 'bind', status: 'succeeded', startedAt: 10, finishedAt: 11, error: null, result: { exitCode: 0, stdout: 'applied v5', stderr: '' } })
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
    const operation: Operation = { apiVersion: 'v1', id: 'op-run', deployment: 'comparison', kind: 'run-action', destructive: false, status: 'running', startedAt: 10, finishedAt: null, error: null, result: null }
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
    vi.spyOn(client, 'operations').mockResolvedValue({ apiVersion: 'v1', operations: [{ apiVersion: 'v1', id: 'op-cli-running', deployment: 'comparison', kind: 'down', destructive: true, status: 'running', startedAt: 5, finishedAt: null, error: null, result: null }], nextCursor: null })
    const cancel = vi.spyOn(client, 'cancel').mockResolvedValue({ apiVersion: 'v1', id: 'op-cli-running', deployment: 'comparison', kind: 'down', destructive: true, status: 'cancelled', startedAt: 5, finishedAt: 6, error: null, result: null })
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
    vi.spyOn(client, 'pollOperation').mockResolvedValue({ apiVersion: 'v1', id: 'op-bind', deployment: 'comparison', kind: 'bind', destructive: false, status: 'failed', startedAt: 10, finishedAt: 11, error: { code: 'route_apply_failed', message: 'provider health check rejected the candidate' }, result: { exitCode: 1, stdout: '', stderr: 'provider unhealthy' } }); vi.spyOn(client, 'routes').mockResolvedValue(failedRoutes)
    render(<App client={client} />); await screen.findByRole('heading', { name: 'comparison', level: 1 }); const lane = screen.getByRole('heading', { name: 'UI consumers' }).parentElement!; await user.click(within(lane).getByRole('button', { name: /ui-feature/ })); await user.selectOptions(screen.getByLabelText('Provider group for ui-feature'), 'base'); await user.click(within(screen.getByRole('dialog', { name: 'Preview complete route replacement' })).getByRole('button', { name: 'Apply complete change' }))
    const result = await screen.findByRole('dialog', { name: 'Connection switch result' }); expect(within(result).getByText('Atomic binding operation failed.')).toBeInTheDocument(); expect(within(result).getByText('route_apply_failed: provider health check rejected the candidate')).toBeInTheDocument(); expect(within(result).getByText(/status failed; transition rolled_back; error provider_unhealthy; rollback recorded at v4/)).toBeInTheDocument()
  })

  it('lists an authored unbound consumer and binds it through the complete change preview', async () => {
    const user = userEvent.setup(); const client = new ApiClient('test'); const fetchMock = vi.mocked(fetch); const spec = deployment.snapshot.spec
    vi.spyOn(client, 'deployment').mockResolvedValue({ ...deployment, snapshot: { spec: { ...spec, instances: [...spec.instances, { name: 'ui-unbound', block: 'web-ui' }], blocks: { 'web-ui': { services: { web: { consumes: { java: { protocol: 'tcp' }, python: { protocol: 'grpc' }, database: { protocol: 'tcp' } } } } } } } } })
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
})
