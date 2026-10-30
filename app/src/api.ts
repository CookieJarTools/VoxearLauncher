// The launcher's commands, typed. Inside the app they go to the Rust core through Tauri. Opened in a plain
// browser (`npm run dev`), they go to an in-memory stand-in instead, so the UI can be previewed anywhere.

import { Channel, invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { open } from '@tauri-apps/plugin-dialog'
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener'
import { mockApi } from './mock'

export type ReleaseChannel = 'stable' | 'beta' | 'nightly'

export interface AppInfo {
  version: string
  platform: string
  home: string
  userHome: string
  settingsFile: string
  defaultLocation: string
}

export interface Install {
  version: string
  channel: ReleaseChannel
  path: string
  located: boolean
  missing: boolean
  installed: string | null
}

export interface Release {
  version: string
  channel: ReleaseChannel
  published: string
  notes: string | null
  size: number
  installed: boolean
  needsNewerLauncher: boolean
}

export interface Releases {
  source: string | null
  releases: Release[]
  stale: string | null
  /** Nothing has been published at the source yet. */
  unpublished: boolean
}

export type UpdateStatus =
  | { kind: 'notSetUp'; reason: string }
  | { kind: 'nothingPublished' }
  | { kind: 'upToDate'; version: string }
  | { kind: 'available'; version: string; notes: string | null; date: string | null }

export type Progress =
  | { phase: 'downloading'; done: number; total: number }
  | { phase: 'verifying' }
  | { phase: 'unpacking'; done: number; total: number }
  | { phase: 'finishing' }

export type ProjectStatus =
  | { kind: 'ready' }
  | { kind: 'engineMissing'; version: string }
  | { kind: 'unpinned' }
  | { kind: 'missing' }
  | { kind: 'unreadable'; reason: string }

export interface Project {
  path: string
  name: string
  engine: string | null
  status: ProjectStatus
  favourite: boolean
  added: string
  opened: string | null
}

export interface Choice {
  id: string
  name: string
  description: string
}

export interface Template extends Choice {
  thumbnail?: string
  setup: string[]
}

export interface Offer {
  format: number
  templates: Template[]
  setup: Choice[]
  ides: Choice[]
}

export interface NewProjectForm {
  name: string
  location: string
  template: string
  setup: string[] | null
  ide: string | null
  git: boolean
  modding: boolean
  steamAppId: number | null
}

/** The launcher's settings file, as stored. */
export interface Settings {
  releases?: string
  trusted_keys?: string[]
  project_location?: string
  launcher_channel?: 'stable' | 'beta'
}

export interface Failure {
  message: string
  cancelled: boolean
}

/** Which way a project would move: `current` is the version it is on. */
export type Direction = 'pin' | 'upgrade' | 'downgrade' | 'current'

export interface UpgradeChoice {
  version: string
  channel: ReleaseChannel
  direction: Direction
  missing: boolean
}

export interface UpgradeChoices {
  name: string
  current: string | null
  choices: UpgradeChoice[]
}

export interface ReleaseNote {
  version: string
  notes: string
}

export type UpgradeEvent =
  | { kind: 'backingUp'; done: number; total: number }
  | { kind: 'step'; message: string }
  | { kind: 'change'; message: string }

export interface Backup {
  archive: string
  project: string
  path: string
  from: string | null
  to: string
  created: string
  size: number
  files: number
  skipped: string[]
  changes: string[]
}

export interface UpgradeReport {
  from: string | null
  to: string
  backup: Backup
  changes: string[]
  /** Whether the engine's own `vox upgrade` ran; when it didn't, only the pin changed. */
  migrated: boolean
}

export interface Api {
  appInfo(): Promise<AppInfo>
  listInstalls(): Promise<Install[]>
  listReleases(): Promise<Releases>
  install(version: string, onProgress: (progress: Progress) => void): Promise<Install>
  cancelInstall(version: string): Promise<void>
  installFile(path: string, onProgress: (progress: Progress) => void): Promise<Install>
  locate(path: string): Promise<Install>
  projectsUsing(version: string): Promise<string[]>
  uninstall(version: string): Promise<{ forgotten: boolean }>
  installSize(path: string): Promise<number>
  listProjects(): Promise<Project[]>
  addProject(path: string): Promise<string>
  removeProject(path: string): Promise<void>
  setFavourite(path: string, favourite: boolean): Promise<void>
  openProject(path: string): Promise<void>
  describeNew(version: string): Promise<Offer>
  checkNewProject(form: NewProjectForm): Promise<string | null>
  createProject(version: string, form: NewProjectForm, onStep: (step: string) => void): Promise<string>
  getSettings(): Promise<Settings>
  saveSettings(settings: Settings): Promise<void>
  upgradeChoices(path: string): Promise<UpgradeChoices>
  releaseNotes(from: string | null, to: string): Promise<ReleaseNote[]>
  upgradeProject(path: string, version: string, allowDowngrade: boolean, onEvent: (event: UpgradeEvent) => void): Promise<UpgradeReport>
  listBackups(path: string | null): Promise<Backup[]>
  restoreBackup(archive: string): Promise<string>
  deleteBackup(archive: string): Promise<void>
  checkLauncherUpdate(): Promise<UpdateStatus>
  installLauncherUpdate(onProgress: (progress: { done: number; total: number | null }) => void): Promise<void>
  restartLauncher(): Promise<void>
  pickFolder(title: string, defaultPath?: string): Promise<string | null>
  pickFile(title: string, name: string, extensions: string[]): Promise<string | null>
  reveal(path: string): Promise<void>
  openUrl(url: string): Promise<void>
  minimize(): void
  toggleMaximize(): void
  close(): void
}

function channel<T>(listener: (value: T) => void): Channel<T> {
  const channel = new Channel<T>()
  channel.onmessage = listener
  return channel
}

const tauriApi: Api = {
  appInfo: () => invoke('app_info'),
  listInstalls: () => invoke('list_installs'),
  listReleases: () => invoke('list_releases'),
  install: (version, onProgress) => invoke('install', { version, onProgress: channel(onProgress) }),
  cancelInstall: (version) => invoke('cancel_install', { version }),
  installFile: (path, onProgress) => invoke('install_file', { path, onProgress: channel(onProgress) }),
  locate: (path) => invoke('locate', { path }),
  projectsUsing: (version) => invoke('projects_using', { version }),
  uninstall: (version) => invoke('uninstall', { version }),
  installSize: (path) => invoke('install_size', { path }),
  listProjects: () => invoke('list_projects'),
  addProject: (path) => invoke('add_project', { path }),
  removeProject: (path) => invoke('remove_project', { path }),
  setFavourite: (path, favourite) => invoke('set_favourite', { path, favourite }),
  openProject: (path) => invoke('open_project', { path }),
  describeNew: (version) => invoke('describe_new', { version }),
  checkNewProject: (form) => invoke('check_new_project', { form }),
  createProject: (version, form, onStep) => invoke('create_project', { version, form, onStep: channel(onStep) }),
  getSettings: () => invoke('get_settings'),
  saveSettings: (settings) => invoke('save_settings', { settings }),
  upgradeChoices: (path) => invoke('upgrade_choices', { path }),
  releaseNotes: (from, to) => invoke('release_notes', { from, to }),
  upgradeProject: (path, version, allowDowngrade, onEvent) =>
    invoke('upgrade_project', { path, version, allowDowngrade, onEvent: channel(onEvent) }),
  listBackups: (path) => invoke('list_backups', { path }),
  restoreBackup: (archive) => invoke('restore_backup', { archive }),
  deleteBackup: (archive) => invoke('delete_backup', { archive }),
  checkLauncherUpdate: () => invoke('check_launcher_update'),
  installLauncherUpdate: (onProgress) => invoke('install_launcher_update', { onProgress: channel(onProgress) }),
  restartLauncher: () => invoke('restart_launcher'),
  pickFolder: async (title, defaultPath) => {
    const picked = await open({ directory: true, title, defaultPath })
    return typeof picked === 'string' ? picked : null
  },
  pickFile: async (title, name, extensions) => {
    const picked = await open({ title, filters: [{ name, extensions }] })
    return typeof picked === 'string' ? picked : null
  },
  reveal: (path) => revealItemInDir(path),
  openUrl: (url) => openUrl(url),
  minimize: () => void getCurrentWindow().minimize(),
  toggleMaximize: () => void getCurrentWindow().toggleMaximize(),
  close: () => void getCurrentWindow().close()
}

export const inTauri = '__TAURI_INTERNALS__' in window

export const api: Api = inTauri ? tauriApi : mockApi

/** Any rejection as a Failure: the Rust side sends one, anything else is wrapped. */
export function failure(error: unknown): Failure {
  if (error && typeof error === 'object' && 'message' in error) {
    const { message, cancelled } = error as Partial<Failure>
    return { message: String(message), cancelled: Boolean(cancelled) }
  }
  return { message: String(error), cancelled: false }
}
