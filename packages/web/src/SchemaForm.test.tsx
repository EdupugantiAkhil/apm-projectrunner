import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import SchemaForm from './SchemaForm'

afterEach(cleanup)

describe('SchemaForm', () => {
  it('generates the supported type matrix, nested objects, required help, and string arrays', async () => {
    const user = userEvent.setup(); const changed = vi.fn()
    render(<SchemaForm schema={{ type: 'object', title: 'Adapter', required: ['name'], properties: { name: { type: 'string', description: 'Stable name' }, count: { type: 'integer' }, ratio: { type: 'number' }, enabled: { type: 'boolean' }, mode: { type: 'string', enum: ['safe', 'fast'] }, tags: { type: 'array', items: { type: 'string' } }, nested: { type: 'object', properties: { path: { type: 'string' } } } } }} onChange={changed} />)
    expect(screen.getByLabelText(/name \*/i)).toBeRequired(); expect(screen.getByText('Stable name')).toBeInTheDocument()
    await user.type(screen.getByLabelText(/name \*/i), 'worker'); await user.type(screen.getByLabelText('count'), '2'); await user.click(screen.getByLabelText('enabled')); await user.selectOptions(screen.getByLabelText('mode'), 'fast'); fireEvent.change(screen.getByLabelText(/^tags/), { target: { value: 'one\ntwo' } }); await user.type(screen.getByLabelText('path'), '/ready')
    expect(changed).toHaveBeenLastCalledWith(expect.objectContaining({ name: 'worker', count: 2, enabled: true, mode: 'fast', tags: ['one', 'two'], nested: { path: '/ready' } }), true)
  })

  it('degrades unsupported constructs to a labeled JSON editor with syntax validation', () => {
    const changed = vi.fn()
    render(<SchemaForm schema={{ title: 'Choice adapter', oneOf: [{ type: 'string' }, { type: 'number' }] }} onChange={changed} />)
    const editor = screen.getByLabelText('Unsupported schema configuration'); fireEvent.change(editor, { target: { value: '{bad' } })
    expect(screen.getByRole('alert')).toHaveTextContent('Enter valid JSON')
    fireEvent.change(editor, { target: { value: '{"mode":"safe"}' } })
    expect(changed).toHaveBeenLastCalledWith({ mode: 'safe' }, true)
  })
})
