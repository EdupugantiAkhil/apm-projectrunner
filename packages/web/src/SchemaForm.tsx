import { useEffect, useId, useState } from 'react'
import type { JsonSchema } from './api'

export interface SchemaFormProps { schema: JsonSchema; value?: Record<string, unknown>; onChange?: (value: Record<string, unknown>, valid: boolean) => void; readOnly?: boolean }
const EMPTY_VALUE: Record<string, unknown> = {}

const supported = (schema: JsonSchema): boolean => {
  if (schema.oneOf || schema.anyOf || schema.allOf || Array.isArray(schema.type)) return false
  if (schema.type === 'object') return Object.values(schema.properties ?? {}).every(supported)
  if (schema.type === 'array') return schema.items?.type === 'string' && !schema.items.enum
  return ['string', 'number', 'integer', 'boolean'].includes(schema.type ?? 'string') || Boolean(schema.enum)
}

function validity(schema: JsonSchema, value: unknown): boolean {
  if (!supported(schema)) { try { JSON.parse(String(value ?? '{}')); return true } catch { return false } }
  if (schema.type === 'object') return (schema.required ?? []).every((name) => { const found = (value as Record<string, unknown> | undefined)?.[name]; return found !== undefined && found !== '' }) && Object.entries(schema.properties ?? {}).every(([name, child]) => validity(child, (value as Record<string, unknown> | undefined)?.[name]))
  if (schema.type === 'integer') return value === undefined || Number.isInteger(value)
  if (schema.type === 'number') return value === undefined || typeof value === 'number'
  if (schema.type === 'array') return value === undefined || Array.isArray(value)
  return true
}

export default function SchemaForm({ schema, value = EMPTY_VALUE, onChange, readOnly = false }: SchemaFormProps) {
  const [draft, setDraft] = useState<Record<string, unknown>>(value)
  useEffect(() => setDraft(value), [value])
  const update = (next: Record<string, unknown>) => { setDraft(next); onChange?.(next, validity(schema, next)) }
  if (!supported(schema)) return <Fallback schema={schema} value={draft} readOnly={readOnly} onChange={(next) => { setDraft(next); onChange?.(next, true) }} />
  return <fieldset className="schema-form" disabled={readOnly}><legend>{schema.title ?? 'Configuration'}</legend>{schema.description && <p className="help">{schema.description}</p>}<ObjectFields schema={schema} value={draft} path="schema" update={update} required={schema.required ?? []} /></fieldset>
}

function ObjectFields({ schema, value, path, update, required }: { schema: JsonSchema; value: Record<string, unknown>; path: string; update: (value: Record<string, unknown>) => void; required: string[] }) {
  return <>{Object.entries(schema.properties ?? {}).map(([name, child]) => <Field key={name} name={name} schema={child} value={value[name]} path={`${path}-${name}`} required={required.includes(name)} update={(next) => update({ ...value, [name]: next })} />)}</>
}

function Field({ name, schema, value, path, required, update }: { name: string; schema: JsonSchema; value: unknown; path: string; required: boolean; update: (value: unknown) => void }) {
  const generated = useId(); const id = `${path}-${generated}`; const label = schema.title ?? name
  if (schema.type === 'object') return <fieldset><legend>{label}{required ? ' *' : ''}</legend>{schema.description && <p className="help">{schema.description}</p>}<ObjectFields schema={schema} value={(value as Record<string, unknown>) ?? {}} path={path} required={schema.required ?? []} update={update} /></fieldset>
  if (schema.type === 'boolean') return <label className="check"><input id={id} type="checkbox" checked={Boolean(value ?? schema.default)} onChange={(event) => update(event.target.checked)} />{label}{required ? ' *' : ''}<Help schema={schema} /></label>
  if (schema.enum) return <label htmlFor={id}>{label}{required ? ' *' : ''}<select id={id} required={required} value={String(value ?? schema.default ?? '')} onChange={(event) => update(event.target.value)}><option value="">Choose</option>{schema.enum.map((option) => <option key={String(option)} value={String(option)}>{String(option)}</option>)}</select><Help schema={schema} /></label>
  if (schema.type === 'array') return <label htmlFor={id}>{label}{required ? ' *' : ''}<textarea id={id} value={Array.isArray(value) ? value.join('\n') : ''} onChange={(event) => update(event.target.value.split('\n').filter(Boolean))} /><span className="help">One value per line. {schema.description}</span></label>
  const numeric = schema.type === 'number' || schema.type === 'integer'
  return <label htmlFor={id}>{label}{required ? ' *' : ''}<input id={id} required={required} type={numeric ? 'number' : 'text'} step={schema.type === 'integer' ? 1 : numeric ? 'any' : undefined} value={String(value ?? schema.default ?? '')} onChange={(event) => update(numeric && event.target.value !== '' ? Number(event.target.value) : event.target.value)} /><Help schema={schema} /></label>
}

function Help({ schema }: { schema: JsonSchema }) { return schema.description ? <span className="help">{schema.description}</span> : null }

function Fallback({ schema, value, onChange, readOnly }: { schema: JsonSchema; value: Record<string, unknown>; onChange: (value: Record<string, unknown>) => void; readOnly: boolean }) {
  const [text, setText] = useState(JSON.stringify(value, null, 2)); const [error, setError] = useState('')
  return <fieldset className="schema-form"><legend>{schema.title ?? 'Configuration'} (JSON)</legend><label>Unsupported schema configuration<textarea readOnly={readOnly} aria-invalid={Boolean(error)} value={text} onChange={(event) => { const next = event.target.value; setText(next); try { const parsed = JSON.parse(next); setError(''); onChange(parsed) } catch { setError('Enter valid JSON') } }} /></label>{error && <p role="alert">{error}</p>}</fieldset>
}
