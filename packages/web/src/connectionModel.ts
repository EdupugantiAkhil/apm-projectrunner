export interface ConnectionSpec {
  instances?: Array<{ name: string; block?: string }>
  blocks?: Record<string, { services?: Record<string, { consumes?: Record<string, unknown> }> }>
  groups?: Record<string, { extends?: string; providers?: Record<string, string> }>
  bindings?: Record<string, string>
  routes?: Record<string, Record<string, string>>
  uiRoutes?: Record<string, unknown>
}

export function resolvedGroups(groups?: ConnectionSpec['groups']) {
  const source = groups ?? {}; const result: Record<string, Record<string, string>> = {}
  const resolve = (name: string, seen = new Set<string>()): Record<string, string> => { if (seen.has(name)) return {}; seen.add(name); const group = source[name]; return group ? { ...(group.extends ? resolve(group.extends, seen) : {}), ...(group.providers ?? {}) } : {} }
  for (const name of Object.keys(source)) result[name] = resolve(name)
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
  const groups = resolvedGroups(spec.groups); const bindings = spec.bindings ?? {}; const direct = spec.routes ?? {}; const result: ActiveConnection[] = []
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
