import { api } from '../api'
import { useStore, type View } from '../store'
import { phaseText, ProgressBar } from './Bits'
import { Icon, Logo, type IconName } from './Icon'

export function TitleBar() {
  const setPalette = useStore((s) => s.setPalette)
  return (
    <header className="titlebar" data-tauri-drag-region>
      <div className="brand" data-tauri-drag-region>
        <Logo size={20} />
        <span data-tauri-drag-region>
          Voxear <span className="brand-sub">Launcher</span>
        </span>
      </div>
      <button className="tb-search" onClick={() => setPalette(true)}>
        <Icon name="search" size={14} />
        Search projects and actions
        <kbd>Ctrl K</kbd>
      </button>
      <div className="tb-spacer" data-tauri-drag-region />
      <div className="tb-win">
        <button title="Minimise" onClick={api.minimize}>
          <Icon name="winMin" size={13} />
        </button>
        <button title="Maximise" onClick={api.toggleMaximize}>
          <Icon name="winMax" size={11} />
        </button>
        <button className="close" title="Close" onClick={api.close}>
          <Icon name="close" size={13} />
        </button>
      </div>
    </header>
  )
}

const NAV: { view: View; label: string; icon: IconName; key: string }[] = [
  { view: 'projects', label: 'Projects', icon: 'grid', key: '1' },
  { view: 'installs', label: 'Installs', icon: 'cube', key: '2' }
]

export function Sidebar() {
  const view = useStore((s) => s.view)
  const setView = useStore((s) => s.setView)
  const projects = useStore((s) => s.projects.length)
  const installs = useStore((s) => s.installs.length)
  const jobs = useStore((s) => s.jobs)
  const version = useStore((s) => s.info?.version)
  const running = jobs.find((job) => job.state === 'running')
  const waiting = jobs.filter((job) => job.state === 'queued').length

  const counts: Record<string, number> = { projects, installs }
  return (
    <nav className="sidebar">
      {NAV.map((item) => (
        <button
          key={item.view}
          className={`nav-item ${view === item.view ? 'active' : ''}`}
          onClick={() => setView(item.view)}
          title={`${item.label} (Ctrl ${item.key})`}
        >
          <Icon name={item.icon} size={16} />
          {item.label}
          <span className="nav-count">{counts[item.view] || ''}</span>
        </button>
      ))}
      <div className="nav-gap" />
      {running && (
        <button className="job-mini" onClick={() => setView('installs')}>
          <div className="job-mini-title">
            <span>{running.label}</span>
            <span>{waiting ? `+${waiting}` : ''}</span>
          </div>
          <ProgressBar progress={running.progress} />
          <span className="hint">{phaseText(running.progress, false).split(' · ')[0]}</span>
        </button>
      )}
      <button className={`nav-item ${view === 'settings' ? 'active' : ''}`} onClick={() => setView('settings')} title="Settings (Ctrl ,)">
        <Icon name="settings" size={16} />
        Settings
      </button>
      <div className="sidebar-foot">Launcher {version ?? ''}</div>
    </nav>
  )
}
