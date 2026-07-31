export interface ConnectionSpec {
  instances?: Array<{ name: string; block?: string; address?: string }>
  blocks?: Record<string, { services?: Record<string, object> }>
  groups?: Record<string, { instances?: string[]; disabled?: string[]; address?: string }>
  bindings?: Record<string, string>
  routes?: Record<string, Record<string, string>>
}

export function resolvedGroups(spec: ConnectionSpec) {
  return Object.fromEntries(
    Object.entries(spec.groups ?? {}).map(([name, group]) => [
      name,
      (group.instances ?? []).filter((member) => {
        const instance = member.split('/', 1)[0]
        return !(group.disabled ?? []).includes(instance)
      }),
    ]),
  ) as Record<string, string[]>
}

export function connectionConsumers(spec: ConnectionSpec) {
  return (spec.instances ?? []).map((instance) => instance.name)
}

export type ActiveConnection = { group: string; member: string; disabled: boolean }

export function activeConnections(spec: ConnectionSpec, instance: string) {
  const result: ActiveConnection[] = []
  for (const [groupName, group] of Object.entries(spec.groups ?? {})) {
    for (const member of group.instances ?? []) {
      if (member.split('/', 1)[0] === instance) {
        result.push({ group: groupName, member, disabled: (group.disabled ?? []).includes(instance) })
      }
    }
  }
  const selected = spec.bindings?.[instance]
  if (selected && !result.some((connection) => connection.group === selected)) {
    result.push({ group: selected, member: instance, disabled: false })
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
