import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { ApiClient, captureTokenFromFragment, type OperationEvent } from './api'

class MockEventSource extends EventTarget {
  static instances: MockEventSource[] = []
  readonly url: string
  onerror: ((event: Event) => void) | null = null
  closed = false
  constructor(url: string | URL) { super(); this.url = String(url); MockEventSource.instances.push(this) }
  close() { this.closed = true }
  emit(kind: string, data: OperationEvent, lastEventId: string) {
    this.dispatchEvent(new MessageEvent(kind, { data: JSON.stringify(data), lastEventId }))
  }
}

describe('ApiClient', () => {
  beforeEach(() => { MockEventSource.instances = []; vi.stubGlobal('EventSource', MockEventSource) })
  afterEach(() => vi.unstubAllGlobals())

  it('captures the fragment token, strips it immediately, and sends bearer auth', async () => {
    window.history.replaceState(null, '', '/gui/#token=fragment-secret')
    const replace = vi.spyOn(window.history, 'replaceState')
    expect(captureTokenFromFragment()).toBe('fragment-secret')
    expect(window.location.hash).toBe('')
    expect(replace).toHaveBeenCalledWith(null, '', '/gui/')
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({ apiVersion: 'v1', deployments: [] }), { status: 200, headers: { 'content-type': 'application/json' } }))
    vi.stubGlobal('fetch', fetchMock)
    await new ApiClient().deployments()
    expect(fetchMock.mock.calls[0][1].headers.authorization).toBe('Bearer fragment-secret')
  })

  it('uses query auth for EventSource and tracks Last-Event-ID for native resume', () => {
    const received: OperationEvent[] = []
    const subscription = new ApiClient('event-token').subscribe('op/1', (event) => received.push(event))
    const source = MockEventSource.instances[0]
    expect(source.url).toContain('/operations/op%2F1/events?access_token=event-token')
    source.emit('build', { id: 7, operationId: 'op/1', kind: 'build', timestamp: 1, data: { message: 'built' } }, '7')
    expect(received).toHaveLength(1)
    expect(subscription.lastEventId).toBe('7')
    subscription.close()
    expect(source.closed).toBe(true)
  })

  it('validates before updating an optimistic deployment definition', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ apiVersion: 'v1', name: 'demo', valid: true, diagnostics: [], preview: {} }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ apiVersion: 'v1', name: 'demo', path: '/demo.yaml', yaml: 'next', hash: 'new' }), { status: 200 }))
    vi.stubGlobal('fetch', fetchMock)
    await new ApiClient('token').updateDefinitionValidated('demo', 'next', 'old')
    expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/deployments'); expect(fetchMock.mock.calls[0][1].body).toBe(JSON.stringify({ name: 'demo', yaml: 'next', validateOnly: true }))
    expect(fetchMock.mock.calls[1][0]).toBe('/api/v1/deployments/demo/definition'); expect(fetchMock.mock.calls[1][1].body).toBe(JSON.stringify({ yaml: 'next', expectedHash: 'old' }))
  })

  it('uses encoded versioned device endpoints', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify([]), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ name: 'build host' }), { status: 200 }))
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
    vi.stubGlobal('fetch', fetchMock)
    const client = new ApiClient('token')
    await client.devices(); await client.checkDevice('build host'); await client.removeDevice('build host')
    expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/devices')
    expect(fetchMock.mock.calls[1][0]).toBe('/api/v1/devices/build%20host/check')
    expect(fetchMock.mock.calls[2][0]).toBe('/api/v1/devices/build%20host')
  })

  it('lists durable operations with encoded filters and a cursor', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({ apiVersion: 'v1', operations: [], nextCursor: null }), { status: 200 }))
    vi.stubGlobal('fetch', fetchMock)
    await new ApiClient('token').operations({ deployment: 'shared app', instance: 'api/one', kind: 'cleanup', status: 'failed', cursor: 'op/2' })
    expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/operations?deployment=shared+app&instance=api%2Fone&kind=cleanup&status=failed&cursor=op%2F2')
    expect(fetchMock.mock.calls[0][1].method).toBeUndefined()
  })

  it('uses versioned profile selection, review, validation, import, and removal contracts', async () => {
    const responses = [
      { apiVersion: 'v1', profiles: [], sourceErrors: [] },
      { apiVersion: 'v1', definition: {} },
      { apiVersion: 'v1', manifest: 'version: 1', reviewHash: 'hash' },
      { apiVersion: 'v1', valid: true, expandedServices: [], diagnostics: [], error: null },
      { apiVersion: 'v1', name: 'api' },
    ]
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(new Response(JSON.stringify(responses.shift() ?? null), { status: responses.length === 0 ? 200 : 200 })))
    vi.stubGlobal('fetch', fetchMock)
    const client = new ApiClient('token'); const origin = { kind: 'discovered-in-source' as const, source: 'shared app', commit: null }; const profile = { apiVersion: 'v1', name: 'api/worker', deployment: 'demo app', origin, trust: 'not-imported' as const, shadowed: false, services: [] }
    await client.profiles(); await client.profile(profile.name, profile.deployment, origin); await client.profileManifest(profile.name, origin.source); await client.validateProfile(profile, 'checkout one'); await client.importProfile(profile.name, origin.source, 'review/hash')
    vi.mocked(fetch).mockResolvedValueOnce(new Response(null, { status: 204 })); await client.removeProfile(profile.name)
    expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/profiles')
    expect(fetchMock.mock.calls[1][0]).toBe('/api/v1/profiles/api%2Fworker?deployment=demo+app&origin=discovered-in-source&source=shared+app')
    expect(fetchMock.mock.calls[2][0]).toBe('/api/v1/profiles/api%2Fworker/manifest?source=shared%20app')
    expect(fetchMock.mock.calls[3][1].body).toBe(JSON.stringify({ deployment: 'demo app', origin, checkout: 'checkout one' }))
    expect(fetchMock.mock.calls[4][1].body).toBe(JSON.stringify({ source: 'shared app', reviewedManifestHash: 'review/hash' }))
    expect(fetchMock.mock.calls[5][1].method).toBe('DELETE')
  })

  it('deregisters sources with an encoded DELETE and no body', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(null, { status: 204 }))
    vi.stubGlobal('fetch', fetchMock)
    await new ApiClient('token').deregisterSource('shared app')
    expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/sources/shared%20app')
    expect(fetchMock.mock.calls[0][1].method).toBe('DELETE')
    expect(fetchMock.mock.calls[0][1].body).toBeUndefined()
  })
})
