import type { MouseEvent } from 'react'
import { create } from 'zustand'
import { api, failure, type AppInfo, type Install, type Progress, type Project, type Releases, type UpdateStatus } from './api'
import type { IconName } from './components/Icon'

export type View = 'projects' | 'installs' | 'settings'

/** An install waiting or running. Installs run one at a time, as the core's lock requires. */
export interface Job {
  /** The version to install, the archive's path, or `<version>+<module>`. */
  key: string
  label: string
  kind: 'release' | 'file' | 'module'
  state: 'queued' | 'running' | 'failed'
  progress: Progress | null
  error?: string
  /** For a release: the optional modules to leave out. */
  without?: string[]
  /** For a module: which version it goes into, which module, and what it's called. */
  version?: string
  module?: string
  title?: string
}

export interface Toast {
  id: number
  kind: 'ok' | 'error' | 'info'
  message: string
  action?: { label: string; run: () => void }
}

/** The launcher's own update: what the last check found, and how far installing it has got. */
export type LauncherUpdate =
  | { state: 'idle'; status: UpdateStatus | null; checkedAt: number | null; error?: string }
  | { state: 'checking' }
  | { state: 'downloading'; version: string; done: number; total: number | null }
  | { state: 'ready'; version: string }

export type MenuEntry = { label: string; icon?: IconName; danger?: boolean; run: () => void } | 'divider'

interface ConfirmRequest {
  title: string
  message: string
  confirm: string
  danger?: boolean
  resolve: (ok: boolean) => void
}

interface State {
  view: View
  info: AppInfo | null
  installs: Install[]
  projects: Project[]
  loaded: boolean
  releases: Releases | null
  releasesError: string | null
  releasesLoading: boolean
  jobs: Job[]
  toasts: Toast[]
  newProject: { open: boolean; version?: string }
  installSheet: boolean
  palette: boolean
  confirmRequest: ConfirmRequest | null
  menu: { x: number; y: number; items: MenuEntry[] } | null
  /** A project to highlight once, such as one just created. */
  highlight: string | null
  /** The project whose engine version is being changed. */
  upgrading: Project | null
  /** The project whose backups are shown. */
  backupsOf: Project | null
  launcherUpdate: LauncherUpdate

  setView(view: View): void
  init(): Promise<void>
  refresh(): Promise<void>
  loadReleases(): Promise<void>
  queueInstall(version: string, without?: string[]): void
  queueModule(version: string, name: string, title: string): void
  retryJob(key: string): void
  queueFile(path: string): void
  cancelJob(key: string): void
  dismissJob(key: string): void
  toast(kind: Toast['kind'], message: string, action?: Toast['action']): void
  dismissToast(id: number): void
  confirm(request: Omit<ConfirmRequest, 'resolve'>): Promise<boolean>
  openMenu(x: number, y: number, items: MenuEntry[]): void
  closeMenu(): void
  openNewProject(version?: string): void
  closeNewProject(): void
  setInstallSheet(open: boolean): void
  setPalette(open: boolean): void
  setHighlight(path: string | null): void
  openUpgrade(project: Project | null): void
  openBackups(project: Project | null): void
  checkLauncherUpdate(): Promise<void>
  installLauncherUpdate(): Promise<void>
}

let nextToast = 1

export const useStore = create<State>((set, get) => {
  const runQueue = async () => {
    const { jobs } = get()
    if (jobs.some((job) => job.state === 'running')) return
    const job = jobs.find((candidate) => candidate.state === 'queued')
    if (!job) return
    const update = (patch: Partial<Job>) =>
      set((state) => ({ jobs: state.jobs.map((j) => (j.key === job.key ? { ...j, ...patch } : j)) }))
    update({ state: 'running', progress: null })
    const onProgress = (progress: Progress) => update({ progress })
    try {
      if (job.kind === 'module') {
        await api.addModule(job.version!, job.module!, onProgress)
        set((state) => ({ jobs: state.jobs.filter((j) => j.key !== job.key) }))
        get().toast('ok', `Added the ${job.title} to Voxear ${job.version}`)
      } else {
        const install =
          job.kind === 'release' ? await api.install(job.key, job.without ?? [], onProgress) : await api.installFile(job.key, onProgress)
        set((state) => ({ jobs: state.jobs.filter((j) => j.key !== job.key) }))
        get().toast('ok', `Installed Voxear ${install.version}`, {
          label: 'New project',
          run: () => get().openNewProject(install.version)
        })
      }
      void get().refresh()
      void get().loadReleases()
    } catch (error) {
      const { message, cancelled } = failure(error)
      if (cancelled) {
        set((state) => ({ jobs: state.jobs.filter((j) => j.key !== job.key) }))
        get().toast('info', `Cancelled installing ${job.label}`)
      } else {
        update({ state: 'failed', error: message })
        get().toast('error', `Couldn't install ${job.label}: ${message}`)
      }
    }
    void runQueue()
  }

  const enqueue = (job: Job) => {
    if (get().jobs.some((existing) => existing.key === job.key && existing.state !== 'failed')) return
    set((state) => ({ jobs: [...state.jobs.filter((j) => j.key !== job.key), job] }))
    void runQueue()
  }

  return {
    view: 'projects',
    info: null,
    installs: [],
    projects: [],
    loaded: false,
    releases: null,
    releasesError: null,
    releasesLoading: false,
    jobs: [],
    toasts: [],
    newProject: { open: false },
    installSheet: false,
    palette: false,
    confirmRequest: null,
    menu: null,
    highlight: null,
    upgrading: null,
    backupsOf: null,
    launcherUpdate: { state: 'idle', status: null, checkedAt: null },

    setView: (view) => set({ view }),

    init: async () => {
      try {
        const info = await api.appInfo()
        set({ info })
      } catch (error) {
        get().toast('error', failure(error).message)
      }
      await get().refresh()
      set({ loaded: true })
      void get().loadReleases()
      // Check for a newer launcher on start and every six hours, quietly.
      void get().checkLauncherUpdate()
      setInterval(() => void get().checkLauncherUpdate(), 6 * 60 * 60 * 1000)
    },

    refresh: async () => {
      try {
        const [installs, projects] = await Promise.all([api.listInstalls(), api.listProjects()])
        set({ installs, projects })
      } catch (error) {
        get().toast('error', failure(error).message)
      }
    },

    loadReleases: async () => {
      set({ releasesLoading: true })
      try {
        set({ releases: await api.listReleases(), releasesError: null })
      } catch (error) {
        set({ releasesError: failure(error).message })
      } finally {
        set({ releasesLoading: false })
      }
    },

    queueInstall: (version, without = []) =>
      enqueue({ key: version, label: `Voxear ${version}`, kind: 'release', state: 'queued', progress: null, without }),

    queueModule: (version, name, title) =>
      enqueue({
        key: `${version}+${name}`,
        label: `the ${title} for Voxear ${version}`,
        kind: 'module',
        state: 'queued',
        progress: null,
        version,
        module: name,
        title
      }),

    retryJob: (key) => {
      const job = get().jobs.find((j) => j.key === key)
      if (job) enqueue({ ...job, state: 'queued', progress: null, error: undefined })
    },

    queueFile: (path) => {
      const name = path.split(/[\\/]/).pop() ?? path
      enqueue({ key: path, label: name, kind: 'file', state: 'queued', progress: null })
    },

    cancelJob: (key) => {
      const job = get().jobs.find((j) => j.key === key)
      if (job?.state === 'running') void api.cancelInstall(key)
      else set((state) => ({ jobs: state.jobs.filter((j) => j.key !== key) }))
    },

    dismissJob: (key) => set((state) => ({ jobs: state.jobs.filter((j) => j.key !== key) })),

    toast: (kind, message, action) => {
      const id = nextToast++
      set((state) => ({ toasts: [...state.toasts.slice(-3), { id, kind, message, action }] }))
      setTimeout(() => get().dismissToast(id), kind === 'error' ? 9000 : action ? 7000 : 4500)
    },

    dismissToast: (id) => set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) })),

    confirm: (request) =>
      new Promise<boolean>((resolve) => {
        set({
          confirmRequest: {
            ...request,
            resolve: (ok) => {
              set({ confirmRequest: null })
              resolve(ok)
            }
          }
        })
      }),

    openMenu: (x, y, items) => set({ menu: { x, y, items } }),
    closeMenu: () => set({ menu: null }),
    openNewProject: (version) => set({ newProject: { open: true, version }, installSheet: false, palette: false }),
    closeNewProject: () => set({ newProject: { open: false } }),
    setInstallSheet: (installSheet) => {
      set({ installSheet, palette: false })
      if (installSheet && !get().releasesLoading) void get().loadReleases()
    },
    setPalette: (palette) => set({ palette }),
    setHighlight: (highlight) => set({ highlight }),
    openUpgrade: (upgrading) => set({ upgrading, palette: false }),
    openBackups: (backupsOf) => set({ backupsOf, palette: false }),

    checkLauncherUpdate: async () => {
      const current = get().launcherUpdate
      if (current.state === 'checking' || current.state === 'downloading' || current.state === 'ready') return
      set({ launcherUpdate: { state: 'checking' } })
      try {
        const status = await api.checkLauncherUpdate()
        set({ launcherUpdate: { state: 'idle', status, checkedAt: Date.now() } })
      } catch (error) {
        set({ launcherUpdate: { state: 'idle', status: null, checkedAt: Date.now(), error: failure(error).message } })
      }
    },

    installLauncherUpdate: async () => {
      const current = get().launcherUpdate
      if (current.state !== 'idle' || current.status?.kind !== 'available') return
      const { version } = current.status
      set({ launcherUpdate: { state: 'downloading', version, done: 0, total: null } })
      try {
        await api.installLauncherUpdate(({ done, total }) => set({ launcherUpdate: { state: 'downloading', version, done, total } }))
        set({ launcherUpdate: { state: 'ready', version } })
      } catch (error) {
        set({ launcherUpdate: { state: 'idle', status: current.status, checkedAt: Date.now(), error: failure(error).message } })
        get().toast('error', failure(error).message)
      }
    }
  }
})

/** Shows a menu under the element that was clicked. */
export function menuAt(event: MouseEvent, items: MenuEntry[]) {
  event.preventDefault()
  event.stopPropagation()
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  const x = event.type === 'contextmenu' ? event.clientX : rect.right
  const y = event.type === 'contextmenu' ? event.clientY : rect.bottom + 4
  useStore.getState().openMenu(x, y, items)
}
