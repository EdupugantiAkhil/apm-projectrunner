import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import App from './App'
import { ApiClient } from './api'

class MockEventSource extends EventTarget {
  onerror: ((event: Event) => void) | null = null
  readonly url: string
  constructor(url: string) { super(); this.url = url }
  close() {}
}

function json(value: unknown, status = 200) { return new Response(JSON.stringify(value), { status, headers: { 'content-type': 'application/json' } }) }
function baseFetch(handler: (body: Record<string, unknown>, attempt: number) => unknown) {
  let attempts = 0
  const operations = new Map<string, unknown>()
  const mock = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input)
    if (url.endsWith('/deployments')) return json({ apiVersion: 'v1', deployments: [] })
    if (url.endsWith('/sources') && (!init?.method || init.method === 'GET')) return json([])
    if (url.endsWith('/devices')) return json([])
    if (url.endsWith('/adapters')) return json([])
    if (url.endsWith('/sources/clone') && init?.method === 'POST') {
      attempts += 1
      const body = JSON.parse(String(init.body)) as Record<string, unknown>
      const terminal = handler(body, attempts)
      const id = `clone-${attempts}`
      operations.set(id, terminal)
      return json({ apiVersion: 'v1', id, deployment: 'source:repo', kind: 'clone', status: 'pending', startedAt: attempts, finishedAt: null, error: null, result: null }, 202)
    }
    const id = url.split('/operations/')[1]
    if (id && !id.includes('/')) return operations.has(id) ? json(operations.get(id)) : json({}, 404)
    throw new Error(`unexpected request ${url}`)
  })
  vi.stubGlobal('fetch', mock)
  vi.stubGlobal('EventSource', MockEventSource)
  return mock
}

async function openSources() {
  render(<App client={new ApiClient('test')} />)
  await userEvent.click(within(await screen.findByRole('navigation', { name: 'Main views' })).getByRole('button', { name: 'sources' }))
}

function terminal(id: number, error: unknown = null) {
  return { apiVersion: 'v1', id: `clone-${id}`, deployment: 'source:repo', kind: 'clone', status: error ? 'failed' : 'succeeded', startedAt: id, finishedAt: id + 1, error, result: error ? null : { exitCode: 0, stdout: '', stderr: '' } }
}

describe('browser Git clone', () => {
  afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks() })

  it('prompts after ambient auth fails and never renders the submitted secret', async () => {
    const secret = 'token-never-rendered'
    const fetchMock = baseFetch((body, attempt) => {
      if (attempt === 1) return terminal(1, { code: 'clone_credentials_required', message: 'Git authentication is required', context: { kind: 'credentials' } })
      expect(body.credentials).toEqual({ username: 'git-user', password: secret })
      return terminal(2)
    })
    await openSources()
    const user = userEvent.setup()
    const cloneForm = screen.getByRole('heading', { name: 'Clone Git repository' }).closest('form')!
    await user.type(within(cloneForm).getByLabelText('Name'), 'repo')
    await user.type(within(cloneForm).getByLabelText('Repository URL'), 'https://git.example/repo.git')
    await user.click(within(cloneForm).getByRole('button', { name: 'Clone repository' }))
    const dialog = await screen.findByRole('dialog', { name: 'Git credentials required' })
    await user.type(within(dialog).getByLabelText('Username'), 'git-user')
    await user.type(within(dialog).getByLabelText('Password or token'), secret)
    expect(document.body).not.toHaveTextContent(secret)
    await user.click(within(dialog).getByRole('button', { name: 'Retry clone' }))
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Git credentials required' })).not.toBeInTheDocument())
    expect(document.body).not.toHaveTextContent(secret)
    expect(fetchMock).toHaveBeenCalled()
  })

  it('requires explicit approval of the displayed SSH fingerprint', async () => {
    const fingerprint = 'SHA256:trusted-host-key'
    const fetchMock = baseFetch((body, attempt) => {
      if (attempt === 1) return terminal(1, { code: 'clone_host_key_approval_required', message: 'the SSH host key requires explicit approval', context: { kind: 'host_key', host: 'git.example', fingerprint } })
      expect(body.approvedHostKey).toEqual({ host: 'git.example', fingerprint })
      return terminal(2)
    })
    await openSources()
    const user = userEvent.setup()
    const cloneForm = screen.getByRole('heading', { name: 'Clone Git repository' }).closest('form')!
    await user.type(within(cloneForm).getByLabelText('Name'), 'repo')
    await user.type(within(cloneForm).getByLabelText('Repository URL'), 'git@git.example:team/repo.git')
    await user.click(within(cloneForm).getByRole('button', { name: 'Clone repository' }))
    const dialog = await screen.findByRole('dialog', { name: 'Approve SSH host key?' })
    expect(within(dialog).getByText(fingerprint)).toBeInTheDocument()
    expect(fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST')).toHaveLength(1)
    await user.click(within(dialog).getByRole('button', { name: 'Approve this fingerprint' }))
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Approve SSH host key?' })).not.toBeInTheDocument())
  })
})
