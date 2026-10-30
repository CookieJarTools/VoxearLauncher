import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { api, failure } from '../api'
import { fuzzy, pretty } from '../lib/format'
import { useStore } from '../store'
import { addProject, openProject } from '../views/projectActions'
import { Icon, type IconName } from './Icon'

interface Command {
  label: string
  hint?: string
  icon: IconName
  run: () => void
}

/** Ctrl+K: every action, project and version in one list. */
export function Palette() {
  const open = useStore((s) => s.palette)
  const setPalette = useStore((s) => s.setPalette)
  const projects = useStore((s) => s.projects)
  const installs = useStore((s) => s.installs)
  const home = useStore((s) => s.info?.userHome)
  const [query, setQuery] = useState('')
  const [active, setActive] = useState(0)
  const listRef = useRef<HTMLDivElement>(null)

  const commands = useMemo<Command[]>(() => {
    const store = useStore.getState()
    const list: Command[] = [
      { label: 'New project', hint: 'Ctrl N', icon: 'plus', run: () => store.openNewProject() },
      { label: 'Add an existing project', icon: 'folderOpen', run: () => void addProject() },
      { label: 'Install a version', icon: 'download', run: () => store.setInstallSheet(true) },
      { label: 'Install the latest stable version', icon: 'download', run: () => store.queueInstall('stable') },
      { label: 'Go to Projects', hint: 'Ctrl 1', icon: 'grid', run: () => store.setView('projects') },
      { label: 'Go to Installs', hint: 'Ctrl 2', icon: 'cube', run: () => store.setView('installs') },
      { label: 'Go to Settings', hint: 'Ctrl ,', icon: 'settings', run: () => store.setView('settings') }
    ]
    for (const project of projects) {
      list.push({ label: `Open ${project.name}`, hint: pretty(project.path, home), icon: 'play', run: () => void openProject(project) })
    }
    for (const install of installs) {
      list.push({
        label: `New project with Voxear ${install.version}`,
        icon: 'cube',
        run: () => store.openNewProject(install.version)
      })
      list.push({
        label: `Show Voxear ${install.version} in files`,
        hint: pretty(install.path, home),
        icon: 'folder',
        run: () => void api.reveal(install.path).catch((e) => store.toast('error', failure(e).message))
      })
    }
    return list
  }, [projects, installs, home])

  const matches = useMemo(() => {
    if (!query.trim()) return commands
    return commands
      .map((command) => ({ command, score: fuzzy(query, command.label) }))
      .filter((match) => match.score >= 0)
      .sort((a, b) => b.score - a.score)
      .map((match) => match.command)
  }, [commands, query])

  useEffect(() => {
    if (open) {
      setQuery('')
      setActive(0)
    }
  }, [open])

  useEffect(() => setActive(0), [query])

  useEffect(() => {
    listRef.current?.querySelector('.palette-item.on')?.scrollIntoView({ block: 'nearest' })
  }, [active])

  if (!open) return null
  const run = (command: Command | undefined) => {
    if (!command) return
    setPalette(false)
    command.run()
  }
  const onKey = (event: KeyboardEvent) => {
    if (event.key === 'ArrowDown') setActive((i) => Math.min(i + 1, matches.length - 1))
    else if (event.key === 'ArrowUp') setActive((i) => Math.max(i - 1, 0))
    else if (event.key === 'Enter') run(matches[active])
    else if (event.key === 'Escape') setPalette(false)
    else return
    event.preventDefault()
  }

  return (
    <div className="overlay top" onMouseDown={() => setPalette(false)}>
      <div className="dialog palette" onMouseDown={(e) => e.stopPropagation()} role="dialog" aria-label="Command palette">
        <div className="palette-input">
          <Icon name="search" size={18} />
          <input autoFocus value={query} placeholder="Search projects and actions" onChange={(e) => setQuery(e.target.value)} onKeyDown={onKey} />
          <kbd>Esc</kbd>
        </div>
        <div className="palette-list" ref={listRef}>
          {matches.length === 0 && <div className="palette-empty">Nothing matches “{query}”</div>}
          {matches.map((command, index) => (
            <button
              key={command.label + index}
              className={`palette-item ${index === active ? 'on' : ''}`}
              onMouseMove={() => setActive(index)}
              onClick={() => run(command)}
            >
              <Icon name={command.icon} size={15} />
              {command.label}
              {command.hint && <span className="hint">{command.hint}</span>}
            </button>
          ))}
        </div>
      </div>
    </div>
  )
}
