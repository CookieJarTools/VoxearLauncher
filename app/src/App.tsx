import { useEffect } from 'react'
import { Sidebar, TitleBar } from './components/Chrome'
import { Confirm, Menu, Toasts } from './components/Overlays'
import { Palette } from './components/Palette'
import { useStore } from './store'
import { InstallSheet } from './views/InstallSheet'
import { InstallsView } from './views/InstallsView'
import { NewProject } from './views/NewProject'
import { ProjectsView } from './views/ProjectsView'
import { SettingsView } from './views/SettingsView'
import { BackupsDialog, UpgradeDialog } from './views/Upgrade'

export default function App() {
  const view = useStore((s) => s.view)

  useEffect(() => {
    void useStore.getState().init()

    const onKey = (event: KeyboardEvent) => {
      const store = useStore.getState()
      const ctrl = event.ctrlKey || event.metaKey
      if (ctrl && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        store.setPalette(!store.palette)
      } else if (ctrl && event.key.toLowerCase() === 'n') {
        event.preventDefault()
        store.openNewProject()
      } else if (ctrl && event.key === '1') {
        event.preventDefault()
        store.setView('projects')
      } else if (ctrl && event.key === '2') {
        event.preventDefault()
        store.setView('installs')
      } else if (ctrl && event.key === ',') {
        event.preventDefault()
        store.setView('settings')
      } else if (event.key === 'F5' || (ctrl && event.key.toLowerCase() === 'r')) {
        // Refreshes the lists instead of reloading the page.
        event.preventDefault()
        void store.refresh()
        void store.loadReleases()
      }
    }
    // The page's own context menu has nothing useful in it, except in text fields.
    const onContextMenu = (event: MouseEvent) => {
      if (!(event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement)) event.preventDefault()
    }
    // Lists change outside the window too: through voxup, or a project folder being moved.
    const onFocus = () => void useStore.getState().refresh()
    window.addEventListener('keydown', onKey)
    window.addEventListener('contextmenu', onContextMenu)
    window.addEventListener('focus', onFocus)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('contextmenu', onContextMenu)
      window.removeEventListener('focus', onFocus)
    }
  }, [])

  return (
    <div className="app">
      <TitleBar />
      <div className="shell">
        <Sidebar />
        <main className="main">
          {view === 'projects' && <ProjectsView />}
          {view === 'installs' && <InstallsView />}
          {view === 'settings' && <SettingsView />}
        </main>
      </div>
      <NewProject />
      <InstallSheet />
      <UpgradeDialog />
      <BackupsDialog />
      <Palette />
      <Confirm />
      <Menu />
      <Toasts />
    </div>
  )
}
