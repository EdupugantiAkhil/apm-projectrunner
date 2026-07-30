export interface ConnectionSpec {
  instances?: Array<{ name: string; block?: string }>
  blocks?: Record<string, { services?: Record<string, { provides?: Record<string, unknown>; consumes?: Record<string, unknown> }> }>
  groups?: Record<string, { extends?: string; instances?: string[] }>
  bindings?: Record<string, string>
  routes?: Record<string, Record<string, string>>
  uiRoutes?: Record<string, unknown>
}

export function resolvedGroups(spec: ConnectionSpec) {
  const source = spec.groups ?? {}; const result: Record<string, Record<string, string>> = {}; const resolvedMembers: Record<string, string[]> = {}
  const capabilities = (reference: string) => { const [instanceName, serviceName] = reference.split('/', 2); const instance = spec.instances?.find((candidate) => candidate.name === instanceName); const block = instance?.block ? spec.blocks?.[instance.block] : undefined; return Array.from(new Set(Object.entries(block?.services ?? {}).filter(([name]) => !serviceName || name === serviceName).flatMap(([, service]) => Object.keys(service.provides ?? {})))) }
  const members = (name: string, seen = new Set<string>()): string[] => { if (resolvedMembers[name]) return resolvedMembers[name]; if (seen.has(name)) return []; const next = new Set(seen); next.add(name); const group = source[name]; if (!group) return []; let inherited = group.extends ? members(group.extends, next) : []; const additions = group.instances ?? []; for (const member of additions) { const provided = new Set(capabilities(member)); inherited = inherited.filter((candidate) => capabilities(candidate).every((capability) => !provided.has(capability))) } return resolvedMembers[name] = [...inherited, ...additions] }
  for (const name of Object.keys(source)) { const providers: Record<string, string> = {}; for (const member of members(name)) for (const capability of capabilities(member)) providers[capability] = member; result[name] = providers }
  return result
}

export function consumedSlots(spec: ConnectionSpec) {
  const result: Record<string, string[]> = {}
  for (const instance of spec.instances ?? []) { const block = instance.block ? spec.blocks?.[instance.block] : undefined; if (!block) continue; const slots = Array.from(new Set(Object.values(block.services ?? {}).flatMap((service) => Object.keys(service.consumes ?? {})))).sort(); if (slots.length) result[instance.name] = slots }
  return result
}

export function connectionConsumers(spec: ConnectionSpec, consumed = consumedSlots(spec)) { return Array.from(new Set([...Object.keys(consumed), ...Object.keys(spec.routes ?? {}), ...Object.keys(spec.bindings ?? {}), ...Object.keys(spec.uiRoutes ?? {})])) }

export type ActiveConnection = { direction: 'consumes' | 'provides'; consumer: string; slot: string; provider: string }

export function activeConnections(spec: ConnectionSpec, instance: string) {
  const groups = resolvedGroups(spec); const bindings = spec.bindings ?? {}; const direct = spec.routes ?? {}; const result: ActiveConnection[] = []
  for (const consumer of connectionConsumers(spec)) {
    const routes = bindings[consumer] && groups[bindings[consumer]] ? groups[bindings[consumer]] : direct[consumer] ?? {}
    for (const [slot, provider] of Object.entries(routes)) {
      if (consumer === instance) result.push({ direction: 'consumes', consumer, slot, provider })
      if (provider === instance || provider.startsWith(`${instance}/`)) result.push({ direction: 'provides', consumer, slot, provider })
    }
  }
  return result
}

export function definitionSpec(preview: Record<string, unknown>): ConnectionSpec | null {
  const definition = preview.definition
  if (!definition || typeof definition !== 'object') return null
  const spec = (definition as { spec?: unknown }).spec
  return spec && typeof spec === 'object' ? spec as ConnectionSpec : null
}

export function updateBindingYaml(yaml: string, consumer: string, group: string) {
  const lines = yaml.split('\n'); const specIndex = lines.findIndex((line) => /^spec:\s*(?:#.*)?$/.test(line)); if (specIndex < 0) throw new Error('Authored definition has no top-level spec mapping.')
  const specEnd = lines.findIndex((line, index) => index > specIndex && /^\S/.test(line)); const end = specEnd < 0 ? lines.length : specEnd; const bindingIndex = lines.findIndex((line, index) => index > specIndex && index < end && /^  bindings:\s*(?:\{\})?\s*(?:#.*)?$/.test(line)); const key = JSON.stringify(consumer); const value = JSON.stringify(group)
  if (bindingIndex < 0) { lines.splice(end, 0, '  bindings:', `    ${key}: ${value}`); return lines.join('\n') }
  if (/\{\}/.test(lines[bindingIndex])) lines[bindingIndex] = '  bindings:'
  const bindingEnd = lines.findIndex((line, index) => index > bindingIndex && index < end && /^(?:  \S|\S)/.test(line)); const limit = bindingEnd < 0 ? end : bindingEnd; const escaped = consumer.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'); const existing = lines.findIndex((line, index) => index > bindingIndex && index < limit && new RegExp(`^    (?:${escaped}|["']${escaped}["']):`).test(line))
  if (existing >= 0) lines[existing] = `    ${key}: ${value}`; else lines.splice(limit, 0, `    ${key}: ${value}`)
  return lines.join('\n')
}
