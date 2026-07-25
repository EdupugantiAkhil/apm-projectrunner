export type OperationStatus = 'pending' | 'running' | 'succeeded' | 'failed' | 'cancelled'
export type CommandKind = 'validate' | 'plan' | 'apply' | 'bind' | 'status' | 'routes' | 'logs' | 'open' | 'down' | 'cleanup' | 'run-action'

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
  instance: string | null
  kind: CommandKind
  destructive: boolean
  status: OperationStatus
  startedAt: number
  finishedAt: number | null
  error: ApiErrorBody | null
  result: { exitCode: number; stdout: string; stderr: string } | null
}
export interface OperationsResponse { apiVersion: string; operations: Operation[]; nextCursor: string | null }
export interface OperationFilters { deployment?: string; instance?: string; kind?: CommandKind; status?: OperationStatus; cursor?: string }
export interface DeploymentSummary {
  name: string
  definitionHash: string | null
  resourceHash: string | null
  appliedAt: number | null
  lastOperation: { id: string; kind: string; status: OperationStatus; startedAt: number; finishedAt: number | null } | null
  customDomains: string[]
  bindings: Record<string, string>
}
export interface ProjectInfo { apiVersion: string; name: string; root: string; registered: boolean }
export interface DeploymentDetail {
  deployment: string
  definitionHash: string | null
  resourceHash: string | null
  appliedAt: number | null
  snapshot: DeploymentSnapshot | null
  manifest: Record<string, unknown> | null
  sourceIdentities: Record<string, SourceIdentity>
  reconciliation: { deployment: string; diagnostics: Array<{ code: string; path: string; message: string }> }
  resources: Array<{ kind: string; id: string; name: string; labels: Record<string, string>; state: string | null; device: string }>
  customDomains: string[]
  bindings: Record<string, string>
}
export interface DeploymentSnapshot { spec?: {
  instances?: Array<{ name: string; block?: string; source?: string; device?: string; parameters?: Record<string, string> }>
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
export type DeviceStatus = 'never' | 'ok' | 'eligible' | 'ineligible' | 'unreachable' | 'auth-failed'
export type DeviceReachability = 'unchecked' | 'reachable' | 'unreachable' | 'auth-failed'
export type DeviceEligibility = 'eligible' | 'ineligible'
export interface DevicePlacement { deployment: string; instance: string }
export interface DeviceRecord {
  name: string
  kind: 'local' | 'ssh'
  host: string | null
  port: number | null
  user: string | null
  identityFile: string | null
  createdAt: number | null
  lastCheckedAt: number | null
  lastCheckStatus: DeviceStatus
  lastCheckDetail: string | null
  reachability: DeviceReachability
  eligibility: DeviceEligibility
  eligibilityReason: string
  placedInstances: DevicePlacement[]
}
export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue }
export interface RouterBinding {
  router: string
  binding: string
  desiredVersion: number | null
  desiredChecksum: string | null
  currentVersion: number | null
  currentChecksum: string | null
  previousVersion: number | null
  previousChecksum: string | null
  observedVersion: number | null
  observedChecksum: string | null
  status: string
  transition: JsonValue
  lastErrorCode: string | null
  updatedAt: number
}
export interface RouteHistory {
  sequence: number
  router: string | null
  binding: string | null
  operationId: string | null
  version: number
  checksum: string
  activationStatus: string
  recordedAt: number
  context: JsonValue
}
export interface RouteState {
  apiVersion: string
  deployment: string
  bindings: RouterBinding[]
  history: RouteHistory[]
}
export interface OperationEvent { id: number; operationId: string; kind: 'operation' | 'build' | 'health' | 'route' | 'log'; timestamp: number; data: Record<string, unknown> }
export interface DeploymentDefinition { apiVersion: string; name: string; path: string; yaml: string; hash: string }
export interface DeploymentValidation { apiVersion: string; name: string; valid: boolean; diagnostics: Array<{ code: string; path: string; message: string }>; preview: Record<string, unknown> }
export interface AdapterRecord { kind: string; declaration: { id?: string; version?: string; capabilities?: string[]; [key: string]: unknown }; configurationSchema: JsonSchema }
export type ProfileTrust = 'trusted' | 'imported' | 'changed' | 'not-imported'
export type ProfileOrigin = { kind: 'project' } | { kind: 'imported-from-source' | 'discovered-in-source'; source: string; commit: string | null }
export interface ProfileRecord { apiVersion: string; name: string; deployment: string; origin: ProfileOrigin; trust: ProfileTrust; shadowed: boolean; services: Array<{ name: string; adapterKind: 'container' | 'script' | 'process-compose' }> }
export interface ProfilesResponse { apiVersion: string; profiles: ProfileRecord[]; sourceErrors: Array<{ source: string; message: string }> }
export interface ProfileDefinition { apiVersion: string; name: string; deployment: string; origin: ProfileOrigin; trust: ProfileTrust; definition: Record<string, unknown> }
export interface ProfileManifestReview { apiVersion: string; source: string; manifest: string; reviewHash: string }
export interface ProfileValidation { apiVersion: string; name: string; deployment: string; checkout: string; valid: boolean; expandedServices: string[]; services: Array<{ name: string; ports: number[]; volumes: Array<{ name: string; target: string; readOnly: boolean }> }>; diagnostics: Array<{ code: string; path: string; message: string }>; error: string | null; draft: string | null }
export interface ProfileValidationInput { targetDeployment?: string; instanceName?: string; device?: string; parameters?: Record<string, string> }
export type StructuredRunCommand = 'up' | 'down' | 'plan' | 'status'
export type RunAction = { name: string; description: string | null; type: 'structured'; command: StructuredRunCommand; overlays?: string[]; variation?: string; set?: string[] } | { name: string; description: string | null; type: 'shell'; command: string }
export interface RunActionsResponse { apiVersion: string; actions: RunAction[]; shellNoticeAcknowledged: boolean }
export interface StructuredRunActionInput { name: string; description?: string; type: 'structured'; command: StructuredRunCommand; overlays?: string[]; variation?: string; set?: string[] }
export type RunActionPreview = {
  apiVersion: string; name: string; description: string
  target: { kind: 'deployment'; name: string; bundle: string } | { kind: 'project-shell-context'; root: string }
  execution: { type: 'structured'; argv: string[] } | { type: 'shell'; command: string }
  shellNoticeAcknowledged: boolean; shellAcknowledgementRequired: boolean; previewHash: string
}
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
  project() { return this.request<ProjectInfo>('/project') }
  deployment(name: string) { return this.request<DeploymentDetail>(`/deployments/${encodeURIComponent(name)}`) }
  routes(name: string) { return this.request<RouteState>(`/deployments/${encodeURIComponent(name)}/routes`) }
  adapters() { return this.request<AdapterRecord[]>('/adapters') }
  profiles() { return this.request<ProfilesResponse>('/profiles') }
  profile(name: string, deployment: string, origin: ProfileOrigin) {
    const query = new URLSearchParams({ deployment, origin: origin.kind })
    if ('source' in origin) query.set('source', origin.source)
    return this.request<ProfileDefinition>(`/profiles/${encodeURIComponent(name)}?${query}`)
  }
  profileManifest(name: string, source: string) { return this.request<ProfileManifestReview>(`/profiles/${encodeURIComponent(name)}/manifest?source=${encodeURIComponent(source)}`) }
  validateProfile(profile: ProfileRecord, checkout: string, input: ProfileValidationInput = {}) { return this.request<ProfileValidation>(`/profiles/${encodeURIComponent(profile.name)}/validate`, { method: 'POST', body: JSON.stringify({ deployment: profile.deployment, origin: profile.origin, checkout, ...input }) }) }
  importProfile(name: string, source: string, reviewedManifestHash: string) { return this.request<ProfileRecord>(`/profiles/${encodeURIComponent(name)}/import`, { method: 'POST', body: JSON.stringify({ source, reviewedManifestHash }) }) }
  removeProfile(name: string) { return this.request<void>(`/profiles/${encodeURIComponent(name)}`, { method: 'DELETE' }) }
  runActions() { return this.request<RunActionsResponse>('/run-actions') }
  createRunAction(action: StructuredRunActionInput) { return this.request<RunActionsResponse>('/run-actions', { method: 'POST', body: JSON.stringify(action) }) }
  updateRunAction(existingName: string, action: StructuredRunActionInput) { return this.request<RunActionsResponse>(`/run-actions/${encodeURIComponent(existingName)}`, { method: 'PUT', body: JSON.stringify(action) }) }
  deleteRunAction(name: string) { return this.request<void>(`/run-actions/${encodeURIComponent(name)}`, { method: 'DELETE' }) }
  previewRunAction(name: string, bundle?: string) { return this.request<RunActionPreview>(`/run-actions/${encodeURIComponent(name)}/execute`, { method: 'POST', body: JSON.stringify(bundle ? { bundle } : {}) }) }
  executeRunAction(name: string, preview: RunActionPreview, acknowledgeShellWarning = false) {
    const bundle = preview.target.kind === 'deployment' ? preview.target.bundle : undefined
    return this.request<Operation>(`/run-actions/${encodeURIComponent(name)}/execute`, { method: 'POST', body: JSON.stringify({ ...(bundle ? { bundle } : {}), confirmed: true, previewHash: preview.previewHash, acknowledgeShellWarning }) })
  }
  definition(name: string) { return this.request<DeploymentDefinition>(`/deployments/${encodeURIComponent(name)}/definition`) }
  validateDeployment(name: string, yaml: string) { return this.request<DeploymentValidation>('/deployments', { method: 'POST', body: JSON.stringify({ name, yaml, validateOnly: true }) }) }
  createDeployment(name: string, yaml: string) { return this.request<DeploymentDefinition>('/deployments', { method: 'POST', body: JSON.stringify({ name, yaml }) }) }
  updateDefinition(name: string, yaml: string, expectedHash: string) { return this.request<DeploymentDefinition>(`/deployments/${encodeURIComponent(name)}/definition`, { method: 'PUT', body: JSON.stringify({ yaml, expectedHash }) }) }
  async updateDefinitionValidated(name: string, yaml: string, expectedHash: string) { await this.validateDeployment(name, yaml); return this.updateDefinition(name, yaml, expectedHash) }
  sources() { return this.request<SourceRecord[]>('/sources') }
  devices() { return this.request<DeviceRecord[]>('/devices') }
  addDevice(device: { name: string; host: string; port: number; user: string; identityFile?: string }) { return this.request<DeviceRecord>('/devices', { method: 'POST', body: JSON.stringify(device) }) }
  removeDevice(name: string) { return this.request<void>(`/devices/${encodeURIComponent(name)}`, { method: 'DELETE' }) }
  checkDevice(name: string) { return this.request<DeviceRecord>(`/devices/${encodeURIComponent(name)}/check`, { method: 'POST' }) }
  registerSource(name: string, path: string) { return this.request<SourceRecord>('/sources', { method: 'POST', body: JSON.stringify({ name, path }) }) }
  deregisterSource(name: string) { return this.request<void>(`/sources/${encodeURIComponent(name)}`, { method: 'DELETE' }) }
  createWorktree(repository: string, ref: string, name?: string, path?: string) {
    return this.request<SourceRecord>('/worktrees', { method: 'POST', body: JSON.stringify({ repository, ref, name: name || undefined, path: path || undefined }) })
  }
  removeWorktree(name: string, allowDirty: boolean) {
    return this.request<{ staged: number; unstaged: number; untracked: number }>(`/worktrees/${encodeURIComponent(name)}`, { method: 'DELETE', body: JSON.stringify({ allowDirty }) })
  }
  command(kind: CommandKind, bundle: string, extra: Record<string, unknown> = {}) {
    return this.request<Operation>(`/commands/${kind}`, { method: 'POST', body: JSON.stringify({ bundle, ...extra }) })
  }
  operations(filters: OperationFilters = {}) {
    const query = new URLSearchParams()
    for (const [name, value] of Object.entries(filters)) if (value) query.set(name, value)
    const suffix = query.size ? `?${query}` : ''
    return this.request<OperationsResponse>(`/operations${suffix}`)
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
