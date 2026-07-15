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
})
