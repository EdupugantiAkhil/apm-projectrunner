export type OperationStatus = 'pending' | 'running' | 'succeeded' | 'failed' | 'cancelled'
export type CommandKind = 'validate' | 'plan' | 'apply' | 'bind' | 'status' | 'routes' | 'logs' | 'open' | 'down' | 'cleanup'

export interface ApiErrorBody { code: string; message: string; context?: unknown }
export class ApiError extends Error {
  readonly status: number
  readonly code: string
  readonly context?: unknown
  constructor(status: number, body: ApiErrorBody) {
    super(body.message)
    this.name = 'ApiError'
    this.status = status
    this.code = body.code
    this.context = body.context
  }
}

export interface Operation {
  apiVersion: string
  id: string
  deployment: string
  kind: CommandKind
  status: OperationStatus
  startedAt: number
  finishedAt: number | null
  error: ApiErrorBody | null
  result: { exitCode: number; stdout: string; stderr: string } | null
}
export interface DeploymentSummary {
  name: string
  definitionHash: string | null
  resourceHash: string | null
  appliedAt: number | null
  lastOperation: { id: string; kind: string; status: OperationStatus; startedAt: number; finishedAt: number | null } | null
  customDomains: string[]
  bindings: Record<string, string>
}
export interface DeploymentDetail {
  deployment: string
  definitionHash: string | null
  resourceHash: string | null
  appliedAt: number | null
  snapshot: DeploymentSnapshot | null
  manifest: Record<string, unknown> | null
  sourceIdentities: Record<string, SourceIdentity>
  reconciliation: { deployment: string; diagnostics: Array<{ code: string; path: string; message: string }> }
  resources: Array<{ kind: string; id: string; name: string; labels: Record<string, string>; state: string | null }>
  customDomains: string[]
  bindings: Record<string, string>
}
export interface DeploymentSnapshot { spec?: {
  instances?: Array<{ name: string; block?: string; source?: string; parameters?: Record<string, string> }>
  blocks?: Record<string, {
    parameters?: Record<string, { required?: boolean; default?: string }>
    services?: Record<string, {
      provides?: Record<string, { protocol?: string; port?: number }>
      consumes?: Record<string, { protocol?: string; address?: { host?: string; port?: number } }>
      execution?: Record<string, unknown>
      probe?: Record<string, unknown>
      publish?: number[]
      volumes?: unknown[]
    }>
  }>
  groups?: Record<string, { extends?: string; providers?: Record<string, string> }>
  bindings?: Record<string, string>
  routes?: Record<string, Record<string, string>>
  uiRoutes?: Record<string, { origin: string; backend: string; downstreamGroup: string }>
  managedProfiles?: Record<string, { route: string; startUrl: string }>
  hostRouter?: Record<string, unknown>
} }
export interface SourceIdentity { path: string; repository?: string | null; ref?: string | null; commit?: string | null; dirty?: boolean | null }
export interface SourceRecord {
  source: { name: string; kind: 'managed' | 'unmanaged'; path: string; requestedRef?: string | null }
  inspection: {
    identity: SourceIdentity
    branch: string | null
    changes: { staged: number; unstaged: number; untracked: number } | null
    ahead: number | null
    behind: number | null
    unknownCode: string | null
  }
}
export interface RouteState {
  deployment: string
  bindings: Array<{ router: string; binding: string; currentVersion: number | null; desiredVersion: number | null; status: string; lastErrorCode: string | null }>
  history: unknown[]
}
export interface OperationEvent { id: number; operationId: string; kind: 'operation' | 'build' | 'health' | 'route' | 'log'; timestamp: number; data: Record<string, unknown> }
export interface DeploymentDefinition { apiVersion: string; name: string; path: string; yaml: string; hash: string }
export interface DeploymentValidation { apiVersion: string; name: string; valid: boolean; diagnostics: Array<{ code: string; path: string; message: string }>; preview: Record<string, unknown> }
export interface AdapterRecord { kind: string; declaration: { id?: string; version?: string; capabilities?: string[]; [key: string]: unknown }; configurationSchema: JsonSchema }
export interface JsonSchema { type?: string | string[]; title?: string; description?: string; enum?: unknown[]; properties?: Record<string, JsonSchema>; required?: string[]; items?: JsonSchema; default?: unknown; oneOf?: unknown[]; anyOf?: unknown[]; allOf?: unknown[]; [key: string]: unknown }

let memoryToken = ''
export function captureTokenFromFragment(location: Location = window.location, history: History = window.history): string {
  const params = new URLSearchParams(location.hash.replace(/^#/, ''))
  const token = params.get('token') ?? ''
  if (token) memoryToken = token
  if (location.hash) history.replaceState(null, '', `${location.pathname}${location.search}`)
  return memoryToken
}

if (typeof window !== 'undefined') captureTokenFromFragment()

export interface EventSubscription { close(): void; readonly lastEventId: string }

export class ApiClient {
  readonly token: string
  private readonly base: string
  constructor(token = memoryToken, base = '/api/v1') { this.token = token; this.base = base }

  private async request<T>(path: string, init: RequestInit = {}): Promise<T> {
    const response = await fetch(`${this.base}${path}`, {
      ...init,
      headers: { 'content-type': 'application/json', ...init.headers, authorization: `Bearer ${this.token}` },
    })
    if (!response.ok) {
      const body = await response.json().catch(() => ({ code: 'http_error', message: response.statusText })) as ApiErrorBody
      throw new ApiError(response.status, body)
    }
    if (response.status === 204) return undefined as T
    return response.json() as Promise<T>
  }

  deployments() { return this.request<{ apiVersion: string; deployments: DeploymentSummary[] }>('/deployments') }
  deployment(name: string) { return this.request<DeploymentDetail>(`/deployments/${encodeURIComponent(name)}`) }
  routes(name: string) { return this.request<RouteState>(`/deployments/${encodeURIComponent(name)}/routes`) }
  adapters() { return this.request<AdapterRecord[]>('/adapters') }
  definition(name: string) { return this.request<DeploymentDefinition>(`/deployments/${encodeURIComponent(name)}/definition`) }
  validateDeployment(name: string, yaml: string) { return this.request<DeploymentValidation>('/deployments', { method: 'POST', body: JSON.stringify({ name, yaml, validateOnly: true }) }) }
  createDeployment(name: string, yaml: string) { return this.request<DeploymentDefinition>('/deployments', { method: 'POST', body: JSON.stringify({ name, yaml }) }) }
  updateDefinition(name: string, yaml: string, expectedHash: string) { return this.request<DeploymentDefinition>(`/deployments/${encodeURIComponent(name)}/definition`, { method: 'PUT', body: JSON.stringify({ yaml, expectedHash }) }) }
  async updateDefinitionValidated(name: string, yaml: string, expectedHash: string) { await this.validateDeployment(name, yaml); return this.updateDefinition(name, yaml, expectedHash) }
  sources() { return this.request<SourceRecord[]>('/sources') }
  registerSource(name: string, path: string) { return this.request<SourceRecord>('/sources', { method: 'POST', body: JSON.stringify({ name, path }) }) }
  createWorktree(repository: string, ref: string, name?: string, path?: string) {
    return this.request<SourceRecord>('/worktrees', { method: 'POST', body: JSON.stringify({ repository, ref, name: name || undefined, path: path || undefined }) })
  }
  removeWorktree(name: string, allowDirty: boolean) {
    return this.request<{ staged: number; unstaged: number; untracked: number }>(`/worktrees/${encodeURIComponent(name)}`, { method: 'DELETE', body: JSON.stringify({ allowDirty }) })
  }
  command(kind: CommandKind, bundle: string, extra: Record<string, unknown> = {}) {
    return this.request<Operation>(`/commands/${kind}`, { method: 'POST', body: JSON.stringify({ bundle, ...extra }) })
  }
  operation(id: string) { return this.request<Operation>(`/operations/${encodeURIComponent(id)}`) }
  cancel(id: string) { return this.request<Operation>(`/operations/${encodeURIComponent(id)}/cancel`, { method: 'POST' }) }

  async pollOperation(id: string, signal?: AbortSignal): Promise<Operation> {
    let delay = 100
    for (;;) {
      if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')
      const operation = await this.operation(id)
      if (['succeeded', 'failed', 'cancelled'].includes(operation.status)) return operation
      await new Promise<void>((resolve, reject) => {
        const timer = window.setTimeout(resolve, delay)
        signal?.addEventListener('abort', () => { window.clearTimeout(timer); reject(new DOMException('Aborted', 'AbortError')) }, { once: true })
      })
      delay = Math.min(delay * 2, 1000)
    }
  }

  subscribe(id: string, onEvent: (event: OperationEvent) => void, onError?: () => void): EventSubscription {
    const url = `${this.base}/operations/${encodeURIComponent(id)}/events?access_token=${encodeURIComponent(this.token)}`
    const source = new EventSource(url)
    let lastEventId = ''
    const receive = (message: MessageEvent<string>) => {
      lastEventId = message.lastEventId || lastEventId
      onEvent(JSON.parse(message.data) as OperationEvent)
    }
    for (const kind of ['operation', 'build', 'health', 'route', 'log']) source.addEventListener(kind, receive as EventListener)
    source.onerror = () => onError?.()
    return { close: () => source.close(), get lastEventId() { return lastEventId } }
  }
}
