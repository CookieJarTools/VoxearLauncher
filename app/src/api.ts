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
}

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
}

export interface Failure {
  message: string
  cancelled: boolean
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
