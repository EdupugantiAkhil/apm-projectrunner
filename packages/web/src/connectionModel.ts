export interface ConnectionSpec {
  instances?: Array<{ name: string; block?: string; address?: string }>
  blocks?: Record<string, { services?: Record<string, object> }>
  groups?: Record<string, { instances?: string[]; disabled?: string[]; address?: string }>
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
  return result
}

export function membershipByInstance(spec: ConnectionSpec) {
  const memberships: Record<string, string> = {}
  for (const [group, definition] of Object.entries(spec.groups ?? {})) {
    for (const member of definition.instances ?? []) memberships[member.split('/', 1)[0]] = group
  }
  return memberships
}

export function definitionSpec(preview: Record<string, unknown>): ConnectionSpec | null {
  const definition = preview.definition
  if (!definition || typeof definition !== 'object') return null
  const spec = (definition as { spec?: unknown }).spec
  return spec && typeof spec === 'object' ? spec as ConnectionSpec : null
}

export function updateGroupInstancesYaml(yaml: string, group: string, members: string[]) {
  const lines = yaml.split('\n')
  const groupsIndex = lines.findIndex((line) => /^  groups:\s*(?:#.*)?$/.test(line))
  if (groupsIndex < 0) throw new Error('Authored definition has no spec.groups mapping.')
  const groupsEnd = lines.findIndex((line, index) => index > groupsIndex && /^(?:  \S|\S)/.test(line))
  const end = groupsEnd < 0 ? lines.length : groupsEnd
  const escaped = group.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const groupIndex = lines.findIndex((line, index) => index > groupsIndex && index < end && new RegExp(`^    (?:${escaped}|["']${escaped}["']):\\s*(?:#.*)?$`).test(line))
  if (groupIndex < 0) throw new Error(`Authored definition has no group ${group}.`)
  const nextGroup = lines.findIndex((line, index) => index > groupIndex && index < end && /^    \S.*:\s*(?:#.*)?$/.test(line))
  const groupEnd = nextGroup < 0 ? end : nextGroup
  const instancesIndex = lines.findIndex((line, index) => index > groupIndex && index < groupEnd && /^      instances:\s*/.test(line))
  const value = JSON.stringify(members)
  if (instancesIndex < 0) {
    lines.splice(groupIndex + 1, 0, `      instances: ${value}`)
    return lines.join('\n')
  }
  lines[instancesIndex] = `      instances: ${value}`
  let removeEnd = instancesIndex + 1
  while (removeEnd < groupEnd && /^        -\s/.test(lines[removeEnd])) removeEnd += 1
  lines.splice(instancesIndex + 1, removeEnd - instancesIndex - 1)
  return lines.join('\n')
}
