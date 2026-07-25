import type { ProfileRecord } from './api'

export const originLabel = (profile: ProfileRecord) => profile.origin.kind === 'project' ? `Project · ${profile.deployment}` : profile.origin.kind === 'imported-from-source' ? `Imported from ${profile.origin.source}` : `Source · ${profile.origin.source}`
export const trustLabel = (trust: ProfileRecord['trust']) => trust === 'changed' ? 'changed — review again' : trust.replace('-', ' ')
