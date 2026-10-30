import { useEffect, useMemo, useRef, useState } from 'react'
import type { Install, Project } from '../api'
import { Icon } from '../components/Icon'
import { ago, fuzzy, hueOf, pretty } from '../lib/format'
import { menuAt, useStore, type MenuEntry } from '../store'
import { addProject, openProject, removeProject, reveal, toggleFavourite } from './projectActions'

export function ProjectsView() {
  const projects = useStore((s) => s.projects)
  const installs = useStore((s) => s.installs)
  const loaded = useStore((s) => s.loaded)
  const home = useStore((s) => s.info?.userHome)
  const highlight = useStore((s) => s.highlight)
  const openNewProject = useStore((s) => s.openNewProject)
  const [query, setQuery] = useState('')
  const [selected, setSelected] = useState<string | null>(null)
  const searchRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)

  const shown = useMemo(() => {
    if (!query.trim()) return projects
    return projects
      .map((project) => ({ project, score: Math.max(fuzzy(query, project.name) * 2, fuzzy(query, project.path)) }))
      .filter((match) => match.score >= 0)
      .sort((a, b) => b.score - a.score)
      .map((match) => match.project)
  }, [projects, query])

  // A project just created or added is selected and scrolled to.
  useEffect(() => {
    if (!highlight) return
    setSelected(highlight)
    setQuery('')
    useStore.getState().setHighlight(null)
  }, [highlight])

  useEffect(() => {
    listRef.current?.querySelector('.row.selected')?.scrollIntoView({ block: 'nearest' })
  }, [selected])

  // `/` searches; arrows move; Enter opens. Only when nothing else has the keyboard.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const state = useStore.getState()
      if (state.palette || state.newProject.open || state.installSheet || state.confirmRequest || state.menu) return
      const typing = event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement
      if (event.key === '/' && !typing) {
        event.preventDefault()
        searchRef.current?.focus()
        return
      }
      if (typing && event.target !== searchRef.current) return
      const index = shown.findIndex((p) => p.path === selected)
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault()
        const next = event.key === 'ArrowDown' ? Math.min(index + 1, shown.length - 1) : Math.max(index - 1, 0)
        setSelected(shown[next]?.path ?? null)
      } else if (event.key === 'Enter') {
        const project = shown[index] ?? (shown.length === 1 ? shown[0] : undefined)
        if (project) void openProject(project)
      } else if (event.key === 'Escape' && event.target === searchRef.current) {
        setQuery('')
        searchRef.current?.blur()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [shown, selected])

  return (
    <section className="view">
      <div className="view-header">
        <h1 className="view-title">
          Projects {projects.length > 0 && <small>{projects.length}</small>}
        </h1>
        <button className="btn" onClick={() => void addProject()}>
          <Icon name="folderOpen" size={15} />
          Add
        </button>
        <button className="btn btn-primary" onClick={() => openNewProject()} title="Ctrl N">
          <Icon name="plus" size={15} strokeWidth={2} />
          New project
        </button>
      </div>

      {projects.length > 0 && (
        <div className="view-tools">
          <label className="search">
            <Icon name="search" size={14} />
            <input ref={searchRef} value={query} placeholder="Search projects" onChange={(e) => setQuery(e.target.value)} />
            {!query && <kbd>/</kbd>}
          </label>
        </div>
      )}

      <div className="view-scroll" ref={listRef}>
        {loaded && projects.length === 0 && <NoProjects hasEngine={installs.length > 0} />}
        {projects.length > 0 && shown.length === 0 && (
          <div className="empty">
            <p>Nothing matches “{query}”.</p>
          </div>
        )}
        <div className="list">
          {shown.map((project, index) => (
            <ProjectRow
              key={project.path}
              project={project}
              index={index}
              install={installs.find((install) => install.version === project.engine)}
              home={home}
              selected={project.path === selected}
              onSelect={() => setSelected(project.path)}
            />
          ))}
        </div>
      </div>
    </section>
  )
}

function ProjectRow({
  project,
  index,
  install,
  home,
  selected,
  onSelect
}: {
  project: Project
  index: number
  install: Install | undefined
  home: string | undefined
  selected: boolean
  onSelect: () => void
}) {
  const menu: MenuEntry[] = [
    { label: 'Open', icon: 'play', run: () => void openProject(project) },
    { label: 'Show in files', icon: 'folder', run: () => void reveal(project.path) },
    { label: project.favourite ? 'Unfavourite' : 'Favourite', icon: 'star', run: () => void toggleFavourite(project) },
    'divider',
    { label: project.engine ? 'Change engine version…' : 'Pin to an engine version…', icon: 'cube', run: () => useStore.getState().openUpgrade(project) },
    { label: 'Backups…', icon: 'archive', run: () => useStore.getState().openBackups(project) },
    'divider',
    { label: 'Remove from list', icon: 'trash', danger: true, run: () => void removeProject(project) }
  ]
  const ready = project.status.kind === 'ready'
  return (
    <div
      className={`row ${selected ? 'selected' : ''}`}
      style={{ animationDelay: `${Math.min(index, 12) * 18}ms` }}
      onClick={onSelect}
      onDoubleClick={() => void openProject(project)}
      onContextMenu={(event) => {
        onSelect()
        menuAt(event, menu)
      }}
    >
      <button
        className={`star ${project.favourite ? 'on' : ''}`}
        title={project.favourite ? 'Unfavourite' : 'Favourite'}
        onClick={(event) => {
          event.stopPropagation()
          void toggleFavourite(project)
        }}
      >
        <Icon name="star" size={14} strokeWidth={1.6} filled={project.favourite} />
      </button>
      <div className="avatar" style={{ '--h': hueOf(project.name) } as React.CSSProperties}>
        {project.name.charAt(0).toUpperCase()}
      </div>
      <div className="row-main">
        <span className="row-name">{project.name}</span>
        <span className="row-path" title={project.path}>
          {pretty(project.path, home)}
        </span>
      </div>
      <EngineChip project={project} install={install} />
      <span className="row-meta" title={project.opened ? 'Last opened' : 'Added'}>
        {ago(project.opened ?? project.added)}
      </span>
      <div className="row-actions">
        {ready && (
          <button
            className="btn btn-sm"
            onClick={(event) => {
              event.stopPropagation()
              void openProject(project)
            }}
          >
            Open
          </button>
        )}
        <button className="icon-btn" title="More" onClick={(event) => menuAt(event, menu)}>
          <Icon name="more" size={18} strokeWidth={2.4} />
        </button>
      </div>
    </div>
  )
}

function EngineChip({ project, install }: { project: Project; install: Install | undefined }) {
  const jobs = useStore((s) => s.jobs)
  const { status } = project
  switch (status.kind) {
    case 'ready':
      return (
        <span className="chip" title={`Voxear ${project.engine}`}>
          <span className="dot" style={{ color: `var(--${install?.channel ?? 'stable'})` }} />
          {project.engine}
        </span>
      )
    case 'engineMissing': {
      const job = jobs.find((j) => j.key === status.version && j.state !== 'failed')
      return (
        <button
          className="chip chip-warn"
          title={`Voxear ${status.version} isn't installed`}
          disabled={Boolean(job)}
          onClick={(event) => {
            event.stopPropagation()
            useStore.getState().queueInstall(status.version)
          }}
        >
          <Icon name={job ? 'download' : 'alert'} size={12} strokeWidth={2} />
          {job ? `Installing ${status.version}` : `Install ${status.version}`}
        </button>
      )
    }
    case 'unpinned':
      return (
        <button
          className="chip chip-faint"
          title="Pin it to an engine version"
          onClick={(event) => {
            event.stopPropagation()
            useStore.getState().openUpgrade(project)
          }}
        >
          Not pinned
        </button>
      )
    case 'missing':
      return <span className="chip chip-warn">Folder not found</span>
    case 'unreadable':
      return (
        <span className="chip chip-danger" title={status.reason}>
          Can't read voxear.toml
        </span>
      )
  }
}

function NoProjects({ hasEngine }: { hasEngine: boolean }) {
  const openNewProject = useStore((s) => s.openNewProject)
  const setInstallSheet = useStore((s) => s.setInstallSheet)
  return (
    <div className="empty">
      <svg className="empty-art" width="120" height="76" viewBox="0 0 160 100" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round">
        <path d="M80 18 L112 36 L80 54 L48 36 Z" />
        <path d="M48 36 V64 L80 82 V54 M112 36 V64 L80 82" />
        <path d="M80 54 L80 82" opacity="0.4" />
        <path d="M126 20 v10 M121 25 h10" stroke="var(--accent)" />
      </svg>
      <h2>No projects yet</h2>
      <p>
        {hasEngine
          ? 'Start one from a template, or add a project you already have.'
          : 'Install an engine version, then start a project from a template. Projects you already have can be added any time.'}
      </p>
      <div className="empty-actions">
        {hasEngine ? (
          <button className="btn btn-primary" onClick={() => openNewProject()}>
            <Icon name="plus" size={15} strokeWidth={2} />
            New project
          </button>
        ) : (
          <button className="btn btn-primary" onClick={() => setInstallSheet(true)}>
            <Icon name="download" size={15} />
            Install a version
          </button>
        )}
        <button className="btn" onClick={() => void addProject()}>
          Add existing
        </button>
      </div>
    </div>
  )
}
