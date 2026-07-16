import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import App from './App'
import { ApiClient, type OperationEvent } from './api'

const deployment = {
  apiVersion: 'v1', deployment: 'comparison', definitionHash: 'definition123', resourceHash: 'resource123', appliedAt: 1,
  snapshot: { spec: { instances: [{ name: 'ui-feature' }, { name: 'backend-a' }, { name: 'backend-b' }, { name: 'python-a' }, { name: 'python-b' }, { name: 'shared-db' }], bindings: { 'ui-feature': 'feature' }, routes: { 'ui-feature': { java: 'backend-a', python: 'python-a', database: 'shared-db' } }, groups: { base: { providers: { java: 'backend-b', python: 'python-b', database: 'shared-db' } }, feature: { providers: { java: 'backend-a', python: 'python-a', database: 'shared-db' } } }, uiRoutes: { browser: { origin: 'https://ui.comparison.localhost', backend: 'backend-a', downstreamGroup: 'feature' } } } }, manifest: {},
  sourceIdentities: { 'ui-feature': { path: '/worktrees/ui-a', ref: 'feature/ui-redesign', commit: '35ad2abcdef', dirty: true } },
  reconciliation: { deployment: 'comparison', diagnostics: [] }, resources: [{ kind: 'container', id: 'one', name: 'comparison-ui-feature', labels: { 'dev.switchyard.instance': 'ui-feature' }, state: 'healthy' }],
  customDomains: ['ui.comparison.localhost'], bindings: { 'ui-feature': 'feature' },
}
const source = { source: { name: 'feature-ui', kind: 'managed', path: '/worktrees/ui-a' }, inspection: { identity: { path: '/worktrees/ui-a', ref: 'feature/ui-redesign', commit: '35ad2abcdef', dirty: true }, branch: 'feature/ui-redesign', changes: { staged: 1, unstaged: 2, untracked: 3 }, ahead: 2, behind: 0, unknownCode: null } }

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
    if (url.endsWith('/deployments') && (!init?.method || init.method === 'GET')) return json({ apiVersion: 'v1', deployments: [{ name: 'comparison', definitionHash: 'definition123', resourceHash: 'resource123', appliedAt: 1, lastOperation: { id: 'old', kind: 'apply', status: 'succeeded', startedAt: 1, finishedAt: 2 }, customDomains: [], bindings: {} }] })
    if (url.endsWith('/deployments/comparison/routes')) return json({ deployment: 'comparison', bindings: [{ router: 'host', binding: 'ui-feature', currentVersion: 4, desiredVersion: 4, status: 'active', lastErrorCode: null }], history: [] })
    if (url.endsWith('/deployments/comparison')) return json(deployment)
    if (url.endsWith('/sources')) return json([source])
    if (url.endsWith('/devices/build-host/check') && init?.method === 'POST') { deviceStatus = 'ok'; return json({ name: 'build-host', host: 'host.test', port: 22, user: 'dev', identityFile: null, createdAt: 1, lastCheckedAt: 1000, lastCheckStatus: 'ok', lastCheckDetail: 'SSH connection succeeded' }) }
    if (url.endsWith('/devices') && init?.method === 'POST') return json({ ...JSON.parse(String(init.body)), identityFile: null, createdAt: 1, lastCheckedAt: null, lastCheckStatus: 'never', lastCheckDetail: null }, 201)
    if (url.endsWith('/devices')) return json([{ name: 'build-host', host: 'host.test', port: 22, user: 'dev', identityFile: null, createdAt: 1, lastCheckedAt: deviceStatus === 'ok' ? 1000 : null, lastCheckStatus: deviceStatus, lastCheckDetail: deviceStatus === 'ok' ? 'SSH connection succeeded' : null }])
    if (url.endsWith('/adapters')) return json([{ kind: 'execution', declaration: { id: 'container', version: '1', capabilities: ['container'] }, configurationSchema: { type: 'object', properties: { type: { type: 'string', enum: ['container'], default: 'container' }, image: { type: 'string' } } } }])
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

  it('renders deployment identity, state, routes, domains, and bindings', async () => {
    render(<App client={new ApiClient('test')} />)
    expect(await screen.findByRole('heading', { name: 'comparison', level: 1 })).toBeInTheDocument()
    expect(screen.getByText('/worktrees/ui-a')).toBeInTheDocument()
    expect(screen.getByText(/35ad2abcd/)).toBeInTheDocument()
    expect(screen.getByText('healthy')).toBeInTheDocument()
    expect(screen.getByText('ui.comparison.localhost')).toBeInTheDocument()
    expect(screen.getByRole('cell', { name: 'v4' })).toBeInTheDocument()
  })

  it('presents a cleaned-up deployment as stopped with a clear Up action and reconciliation reason', async () => {
    const stopped = { ...deployment, resources: [], customDomains: [], reconciliation: { deployment: 'comparison', diagnostics: [{ code: 'observed_resources_missing', path: 'observed.resources', message: 'no labeled Docker resources were observed' }] } }
    vi.mocked(fetch).mockImplementation(async (input: string | URL | Request) => {
      const url = String(input)
      if (url.endsWith('/deployments')) return json({ apiVersion: 'v1', deployments: [{ name: 'comparison', definitionHash: 'definition123', resourceHash: 'resource123', appliedAt: 1, lastOperation: null, customDomains: [], bindings: {} }] })
      if (url.endsWith('/deployments/comparison/routes')) return json({ deployment: 'comparison', bindings: [], history: [] })
      if (url.endsWith('/deployments/comparison')) return json(stopped)
      if (url.endsWith('/sources')) return json([source])
      if (url.endsWith('/adapters')) return json([])
      throw new Error(`unexpected request ${url}`)
    })
    render(<App client={new ApiClient('test')} />)
    expect(await screen.findByText('Deployment is stopped or cleaned up')).toBeInTheDocument()
    expect(screen.getAllByText('Stopped / cleaned up').length).toBeGreaterThan(0)
    expect(screen.getAllByText(/no labeled Docker resources were observed/).length).toBeGreaterThan(0)
    expect(screen.getByRole('button', { name: 'Run Up' })).toBeInTheDocument()
    expect(screen.getAllByText('not running')).toHaveLength(6)
    expect(screen.getByText('Live patch bay unavailable')).toBeInTheDocument()
    expect(screen.queryByRole('img', { name: 'Route cables' })).not.toBeInTheDocument()
    expect(screen.queryByText('state unknown')).not.toBeInTheDocument()
    expect(screen.getByText('Unavailable while stopped')).toBeInTheDocument()
  })

  it('requires an explicit second step before dirty worktree removal', async () => {
    const user = userEvent.setup()
    render(<App client={new ApiClient('test')} />)
    await user.click(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'sources' }))
    await user.click(await screen.findByRole('button', { name: 'Remove' }))
    expect(within(screen.getByRole('dialog')).getByText(/1 staged, 2 unstaged, 3 untracked/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Review dirty removal' }))
    expect(screen.getByText(/Second step/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Confirm removal' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
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

  it('renders devices and refreshes a row after a connection check', async () => {
    const user = userEvent.setup(); render(<App client={new ApiClient('test')} />)
    await user.click(within(screen.getByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'devices' }))
    expect(await screen.findByRole('cell', { name: 'dev@host.test:22' })).toBeInTheDocument()
    expect(screen.getByText('never')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Check connection' }))
    expect(await screen.findByText('ok')).toBeInTheDocument()
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

  it('renders patch lanes and cables and performs a keyboard-only complete binding switch', async () => {
    const user = userEvent.setup(); const fetchMock = vi.mocked(fetch)
    render(<App client={new ApiClient('test')} />); await screen.findByRole('heading', { name: 'comparison', level: 1 })
    expect(screen.getByRole('img', { name: 'Route cables' }).querySelectorAll('path[data-slot]')).toHaveLength(3)
    const lane = screen.getByRole('heading', { name: 'UI consumers' }).parentElement!; await user.click(within(lane).getByRole('button', { name: /ui-feature/ }))
    const select = screen.getByLabelText('Provider group for ui-feature'); select.focus(); await user.selectOptions(select, 'base')
    const dialog = screen.getByRole('dialog', { name: 'Preview complete route replacement' }); expect(within(dialog).getByText(/Snapshot v4/)).toBeInTheDocument(); expect(within(dialog).getAllByRole('row')).toHaveLength(4); expect(within(dialog).getByText('backend-b')).toBeInTheDocument()
    await user.click(within(dialog).getByRole('button', { name: 'Apply complete change' }))
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith('/api/v1/commands/bind', expect.objectContaining({ body: JSON.stringify({ bundle: '.switchyard/generated/comparison/resolved-deployment.yaml', consumer: 'ui-feature', group: 'base', transition: { strategy: 'close' } }) })))
  })

  it('builder validates a schema-driven draft and saves it', async () => {
    const user = userEvent.setup(); const fetchMock = vi.mocked(fetch); render(<App client={new ApiClient('test')} />)
    await user.click(screen.getByRole('button', { name: /New deployment/ })); await user.type(screen.getByLabelText(/^Name/), 'demo'); await user.type(screen.getByLabelText('Instance name'), 'worker'); await user.type(screen.getByLabelText('Block name'), 'service'); await user.selectOptions(screen.getByLabelText('Source'), 'feature-ui')
    await user.click(screen.getByRole('button', { name: 'Validate draft' })); expect(await screen.findByText('Expanded services')).toBeInTheDocument(); await user.click(screen.getByRole('button', { name: 'Save deployment' }))
    await waitFor(() => expect(fetchMock.mock.calls.some(([url, init]) => String(url).endsWith('/deployments') && init?.method === 'POST' && !JSON.parse(String(init.body)).validateOnly)).toBe(true))
  })

  it('shows a domain YAML diff and validates before definition PUT', async () => {
    const user = userEvent.setup(); const fetchMock = vi.mocked(fetch); render(<App client={new ApiClient('test')} />); await screen.findByRole('heading', { name: 'comparison', level: 1 }); await user.click(screen.getByRole('button', { name: 'Load routing definition' })); const editor = await screen.findByLabelText('Deployment YAML'); await user.type(editor, '  hostRouter: {{}}\n'); expect(screen.getByRole('heading', { name: 'Full YAML diff' })).toBeInTheDocument(); await user.click(screen.getByRole('button', { name: 'Validate changes' })); await user.click(await screen.findByRole('button', { name: 'Apply definition edit' }));
    await waitFor(() => { const calls = fetchMock.mock.calls; const put = calls.findIndex(([url, init]) => String(url).endsWith('/definition') && init?.method === 'PUT'); const validation = calls.findIndex(([url, init]) => String(url).endsWith('/deployments') && init?.method === 'POST' && JSON.parse(String(init.body)).validateOnly); expect(validation).toBeGreaterThanOrEqual(0); expect(put).toBeGreaterThan(validation) })
  })
})
