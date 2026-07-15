import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import App from './App'
import { ApiClient, type OperationEvent } from './api'

const deployment = {
  apiVersion: 'v1', deployment: 'comparison', definitionHash: 'definition123', resourceHash: 'resource123', appliedAt: 1,
  snapshot: { spec: { instances: [{ name: 'ui-feature' }], bindings: { 'ui-feature': 'feature' } } }, manifest: {},
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
  const fetchMock = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input)
    if (url.endsWith('/deployments')) return json({ apiVersion: 'v1', deployments: [{ name: 'comparison', definitionHash: 'definition123', resourceHash: 'resource123', appliedAt: 1, lastOperation: { id: 'old', kind: 'apply', status: 'succeeded', startedAt: 1, finishedAt: 2 }, customDomains: [], bindings: {} }] })
    if (url.endsWith('/deployments/comparison/routes')) return json({ deployment: 'comparison', bindings: [{ router: 'host', binding: 'ui-feature', currentVersion: 4, desiredVersion: 4, status: 'active', lastErrorCode: null }], history: [] })
    if (url.endsWith('/deployments/comparison')) return json(deployment)
    if (url.endsWith('/sources')) return json([source])
    if (url.includes('/worktrees/feature-ui')) return json({ staged: 1, unstaged: 2, untracked: 3 })
    if (url.endsWith('/commands/validate') && init?.method === 'POST') return json({ apiVersion: 'v1', id: 'op-new', deployment: 'comparison', kind: 'validate', status: 'running', startedAt: 10, finishedAt: null, error: null, result: null }, 202)
    if (url.endsWith('/operations/op-new')) { operationReads += 1; return json({ apiVersion: 'v1', id: 'op-new', deployment: 'comparison', kind: 'validate', status: 'succeeded', startedAt: 10, finishedAt: 11, error: null, result: { exitCode: 0, stdout: 'valid', stderr: '' } }) }
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
})
