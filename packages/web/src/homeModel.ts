import type { DeploymentDetail, DeploymentSummary, DeviceRecord, Operation, ProfileRecord, SourceRecord } from './api'
import { consumedSlots, resolvedGroups, type ConnectionSpec } from './connectionModel'

export type HomeDestination = 'sources' | 'devices' | 'profiles' | 'builder' | 'deployments' | 'operations'
export type ChecklistState = 'done' | 'todo' | 'unknown'
export interface DeploymentSignal { summary: DeploymentSummary; detail: DeploymentDetail | null; spec: ConnectionSpec | null; error: string | null }
export interface ChecklistStep { label: string; state: ChecklistState; signal: string; action: string; destination: HomeDestination }
export interface Problem { category: 'Sources' | 'Profiles' | 'Deployments' | 'Devices' | 'Operations' | 'Connections'; message: string; destination: HomeDestination }

const trusted = (profile: ProfileRecord) => !profile.shadowed && (profile.trust === 'trusted' || profile.trust === 'imported')
const stateFromSignals = (done: boolean, loading: boolean, unavailable: boolean): ChecklistState => done ? 'done' : loading || unavailable ? 'unknown' : 'todo'

export function missingConnections(spec: ConnectionSpec) {
  const consumed = consumedSlots(spec); const groups = resolvedGroups(spec.groups); const bindings = spec.bindings ?? {}; const direct = spec.routes ?? {}; const missing: Array<{ consumer: string; slots: string[] }> = []
  for (const [consumer, slots] of Object.entries(consumed)) { const group = bindings[consumer]; const routes = group && groups[group] ? groups[group] : direct[consumer] ?? {}; const absent = slots.filter((slot) => !routes[slot]); if (absent.length) missing.push({ consumer, slots: absent }) }
  return missing
}

export function setupChecklist({ sources, profiles, deployments, sourcesLoading, profilesLoading, deploymentsLoading, sourcesUnavailable, profilesUnavailable, deploymentsUnavailable }: { sources: SourceRecord[]; profiles: ProfileRecord[]; deployments: DeploymentSignal[]; sourcesLoading: boolean; profilesLoading: boolean; deploymentsLoading: boolean; sourcesUnavailable: boolean; profilesUnavailable: boolean; deploymentsUnavailable: boolean }): ChecklistStep[] {
  const sourceDone = sources.length > 0; const profileDone = profiles.some(trusted); const unavailable = deploymentsUnavailable || deployments.some((deployment) => Boolean(deployment.error)); const instanceDone = deployments.some((deployment) => !deployment.error && Boolean(deployment.spec?.instances?.length)); const startupDone = deployments.some((deployment) => deployment.summary.appliedAt !== null); const connectionDone = deployments.some((deployment) => !deployment.error && deployment.spec ? Object.keys(consumedSlots(deployment.spec)).length > 0 && missingConnections(deployment.spec).length < Object.keys(consumedSlots(deployment.spec)).length : false)
  return [
    { label: 'Source registered', state: stateFromSignals(sourceDone, sourcesLoading, sourcesUnavailable), signal: 'Registered source records from GET /sources.', action: 'Register a source', destination: 'sources' },
    { label: 'Profile selected', state: stateFromSignals(profileDone, profilesLoading, profilesUnavailable), signal: 'The API has no separate durable selection record; completion means a non-shadowed trusted or imported profile is available for guided authoring.', action: 'Select a startup profile', destination: 'profiles' },
    { label: 'Instance created', state: stateFromSignals(instanceDone, deploymentsLoading, unavailable), signal: 'At least one authored instance is present in a loaded deployment spec.', action: 'Create an instance', destination: 'builder' },
    { label: 'Startup complete', state: stateFromSignals(startupDone, deploymentsLoading, unavailable), signal: 'A deployment summary has a non-null appliedAt timestamp; the API exposes no structured running verdict.', action: 'Start a deployment', destination: 'deployments' },
    { label: 'Connection bound', state: stateFromSignals(connectionDone, deploymentsLoading, unavailable), signal: 'At least one consumer has providers for every consumed slot in its authored spec.', action: 'Bind a connection', destination: 'deployments' },
  ]
}

export function projectProblems({ sources, profileSourceErrors, deployments, devices, operations }: { sources: SourceRecord[]; profileSourceErrors: Array<{ source: string; message: string }>; deployments: DeploymentSignal[]; devices: DeviceRecord[]; operations: Operation[] }): Problem[] {
  const problems: Problem[] = []
  for (const source of sources) if (source.inspection.unknownCode) problems.push({ category: 'Sources', message: `${source.source.name}: inspection unavailable (${source.inspection.unknownCode}).`, destination: 'sources' })
  for (const error of profileSourceErrors) problems.push({ category: 'Profiles', message: `${error.source}: ${error.message}`, destination: 'profiles' })
  for (const deployment of deployments) {
    for (const diagnostic of deployment.detail?.reconciliation.diagnostics ?? []) problems.push({ category: 'Deployments', message: `${deployment.summary.name}: ${diagnostic.message} (${diagnostic.code}).`, destination: 'deployments' })
    if (deployment.spec) for (const missing of missingConnections(deployment.spec)) problems.push({ category: 'Connections', message: `${deployment.summary.name} / ${missing.consumer}: unbound slots ${missing.slots.join(', ')}.`, destination: 'deployments' })
  }
  for (const device of devices) {
    if (device.reachability !== 'reachable') problems.push({ category: 'Devices', message: `${device.name}: reachability is ${device.reachability}.`, destination: 'devices' })
    if (device.eligibility !== 'eligible') problems.push({ category: 'Devices', message: `${device.name}: ${device.eligibilityReason}`, destination: 'devices' })
  }
  for (const operation of operations) if (operation.status === 'failed') problems.push({ category: 'Operations', message: `${operation.deployment} ${operation.kind}: ${operation.error?.message ?? (operation.result?.stderr.trim() || 'operation failed')}.`, destination: 'operations' })
  return problems
}
