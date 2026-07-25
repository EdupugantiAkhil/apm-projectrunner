import { useState } from 'react'
import type { ApiClient, ProfileDefinition, ProfileManifestReview, ProfileRecord, ProfileValidation, SourceRecord } from './api'
import { originLabel, trustLabel } from './profileModel'

const sourceName = (profile: ProfileRecord) => 'source' in profile.origin ? profile.origin.source : null

export default function ProfilesView({ client, profiles, sourceErrors, sources, reload, report }: { client: ApiClient; profiles: ProfileRecord[]; sourceErrors: Array<{ source: string; message: string }>; sources: SourceRecord[]; reload: () => Promise<void>; report: (error: unknown) => void }) {
  const [selected, setSelected] = useState<ProfileRecord | null>(null)
  const [definition, setDefinition] = useState<ProfileDefinition | null>(null)
  const [review, setReview] = useState<ProfileManifestReview | null>(null)
  const [checkout, setCheckout] = useState('')
  const [validation, setValidation] = useState<ProfileValidation | null>(null)
  const [busy, setBusy] = useState('')

  const inspect = async (profile: ProfileRecord) => {
    setSelected(profile); setDefinition(null); setValidation(null); setCheckout('')
    try { setDefinition(await client.profile(profile.name, profile.deployment, profile.origin)) } catch (error) { report(error) }
  }
  const openReview = async (profile: ProfileRecord) => {
    const source = sourceName(profile); if (!source) return
    setSelected(profile); setBusy('review')
    try { setReview(await client.profileManifest(profile.name, source)) } catch (error) { report(error) } finally { setBusy('') }
  }
  const importReviewed = async () => {
    if (!selected || !review) return
    setBusy('import')
    try { await client.importProfile(selected.name, review.source, review.reviewHash); setReview(null); setSelected(null); setDefinition(null); await reload() } catch (error) { report(error) } finally { setBusy('') }
  }
  const removeImported = async (profile: ProfileRecord) => {
    if (!window.confirm(`Remove imported startup profile ${profile.name}? The source manifest and project profiles will not be changed.`)) return
    setBusy(`remove:${profile.name}`)
    try { await client.removeProfile(profile.name); if (selected?.name === profile.name) { setSelected(null); setDefinition(null); setValidation(null) }; await reload() } catch (error) { report(error) } finally { setBusy('') }
  }
  const validate = async () => {
    if (!selected || !checkout) return
    setBusy('validate'); setValidation(null)
    try { setValidation(await client.validateProfile(selected, checkout)) } catch (error) { report(error) } finally { setBusy('') }
  }

  return <section><div className="title-row"><div><p className="eyebrow">Reusable startup definitions</p><h1>Profiles</h1></div></div>
    <section className="profile-boundary" role="note"><h2>Profile editing is not available</h2><p>This library supports discovery, review, import, validation, and removal. Creating or editing profile definitions stays unavailable until the shared operations layer provides a safe mutation.</p></section>
    {sourceErrors.length > 0 && <section className="profile-errors" role="status"><h2>Source manifest problems</h2><ul>{sourceErrors.map((error) => <li key={`${error.source}-${error.message}`}><strong>{error.source}</strong>: {error.message}</li>)}</ul></section>}
    {profiles.length === 0 ? <p>No startup profiles discovered. Add a project profile or register a source containing <span className="mono">switchyard-profiles.yaml</span>.</p> : <div className="profile-grid">{profiles.map((profile) => { const source = sourceName(profile); const importable = profile.trust === 'not-imported' || profile.trust === 'changed'; const imported = profile.origin.kind === 'imported-from-source'; return <article className="profile-card" key={`${profile.deployment}-${profile.origin.kind}-${source ?? 'project'}-${profile.name}`}><header><div><h2>{profile.name}</h2><p><span className="origin-badge">{originLabel(profile)}</span> <span className={`trust-badge trust-${profile.trust}`}>{trustLabel(profile.trust)}</span>{profile.shadowed && <span className="shadow-badge">shadowed</span>}</p></div><button onClick={() => void inspect(profile)}>Inspect</button></header><p>{profile.services.length} service{profile.services.length === 1 ? '' : 's'} · {profile.services.map((service) => `${service.name} (${service.adapterKind})`).join(', ') || 'no services'}</p><div className="profile-actions">{importable && source && <button className="primary" disabled={busy === 'review'} onClick={() => void openReview(profile)}>{profile.trust === 'changed' ? 'Review changed manifest' : 'Review manifest to import'}</button>}{imported && <button className="danger" disabled={busy === `remove:${profile.name}`} onClick={() => void removeImported(profile)}>Remove imported</button>}</div></article> })}</div>}
    {selected && <section className="profile-detail"><h2>Expanded definition · {selected.name}</h2>{definition ? <pre>{JSON.stringify(definition.definition, null, 2)}</pre> : <p className="muted">Loading expanded definition…</p>}<div className="profile-validation"><label>Validate against checkout<select value={checkout} onChange={(event) => { setCheckout(event.target.value); setValidation(null) }}><option value="">Choose checkout</option>{sources.map((source) => <option key={source.source.name}>{source.source.name}</option>)}</select></label><button disabled={!checkout || busy === 'validate'} onClick={() => void validate()}>{busy === 'validate' ? 'Validating…' : 'Validate expansion'}</button></div>{validation && <section className={validation.valid ? 'validation-pass' : 'validation-fail'} role="status"><h3>{validation.valid ? 'Validation passed' : validation.error ? 'Validation could not run' : 'Validation errors'}</h3><p>Profile <strong>{validation.name}</strong> against checkout <strong>{validation.checkout}</strong>.</p>{validation.expandedServices.length > 0 && <><h3>Expanded services</h3><ul>{validation.expandedServices.map((service) => <li className="mono" key={service}>{service}</li>)}</ul></>}{validation.diagnostics.length > 0 && <ul>{validation.diagnostics.map((diagnostic) => <li key={`${diagnostic.code}-${diagnostic.path}`}><strong>{diagnostic.path}</strong> — {diagnostic.message} (<span className="mono">{diagnostic.code}</span>)</li>)}</ul>}{validation.error && <p>{validation.error}</p>}</section>}</section>}
    {review && selected && <div className="modal-backdrop"><div role="dialog" aria-modal="true" aria-labelledby="profile-review-title" className="modal profile-review"><h2 id="profile-review-title">Review and trust import</h2><p>Importing <strong>{selected.name}</strong> trusts the declarative definition from <strong>{review.source}</strong>. No repository script is inferred or executed. Review the verbatim manifest below.</p><pre>{review.manifest}</pre><div><button onClick={() => setReview(null)}>Refuse</button><button className="primary" disabled={busy === 'import'} onClick={() => void importReviewed()}>{selected.trust === 'changed' ? 'Re-import reviewed manifest' : 'Import reviewed manifest'}</button></div></div></div>}
  </section>
}
