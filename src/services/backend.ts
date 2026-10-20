// The only module that talks to the Rust core. Every other module goes through `backend`.

import { invoke } from "@tauri-apps/api/core";
import type {
  AppStatus,
  EntryContent,
  EntryDetails,
  EntryField,
  ErrorKind,
  FileKind,
  GeneratorOptions,
  Icon,
  ImportSummary,
  MergeReport,
  Preferences,
  RevisionSummary,
  Uuid,
  VaultView,
} from "../types";

export class BackendError extends Error {
  constructor(
    readonly kind: ErrorKind,
    message: string,
  ) {
    super(message);
    this.name = "BackendError";
  }
}

function toBackendError(error: unknown): BackendError {
  if (error && typeof error === "object" && "kind" in error && "message" in error) {
    const { kind, message } = error as { kind: ErrorKind; message: string };
    return new BackendError(kind, message);
  }
  return new BackendError("unknown", typeof error === "string" ? error : "Something went wrong");
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw toBackendError(error);
  }
}

export function isCredentialError(error: unknown): boolean {
  return error instanceof BackendError && error.kind === "invalidCredentials";
}

export const backend = {
  // Application
  getStatus: () => call<AppStatus>("get_status"),
  updatePreferences: (preferences: Preferences) => call<AppStatus>("update_preferences", { preferences }),
  forgetRecentVault: (path: string) => call<AppStatus>("forget_recent_vault", { path }),
  touch: () => call<void>("touch"),
  quit: () => call<void>("quit_app"),

  // Vault session
  selectVault: (path: string) => call<AppStatus>("select_vault", { path }),
  closeVault: () => call<AppStatus>("close_vault"),
  createVault: (path: string, name: string, password: string, keyFile: string | null) =>
    call<AppStatus>("create_vault", { path, name, password, keyFile }),
  unlockVault: (password: string, keyFile: string | null) => call<AppStatus>("unlock_vault", { password, keyFile }),
  lockVault: () => call<AppStatus>("lock_vault"),
  generateKeyFile: () => call<string | null>("generate_key_file"),
  changeMasterKey: (password: string, keyFile: string | null) =>
    call<AppStatus>("change_master_key", { password, keyFile }),
  getVaultView: () => call<VaultView>("get_vault_view"),
  renameVault: (name: string) => call<void>("rename_vault", { name }),

  // File dialogs
  pickOpenPath: (kind: FileKind, title: string) => call<string | null>("pick_open_path", { kind, title }),
  pickSavePath: (kind: FileKind, title: string, suggestedName: string | null = null) =>
    call<string | null>("pick_save_path", { kind, title, suggestedName }),

  // Entries
  searchEntries: (query: string) => call<Uuid[]>("search_entries", { query }),
  getEntry: (id: Uuid) => call<EntryDetails>("get_entry", { id }),
  getEntryForEdit: (id: Uuid) => call<EntryContent>("get_entry_for_edit", { id }),
  createEntry: (folderId: Uuid | null, content: EntryContent) => call<Uuid>("create_entry", { folderId, content }),
  updateEntry: (id: Uuid, content: EntryContent) => call<void>("update_entry", { id, content }),
  duplicateEntry: (id: Uuid) => call<Uuid>("duplicate_entry", { id }),
  moveEntry: (id: Uuid, folderId: Uuid | null) => call<void>("move_entry", { id, folderId }),
  deleteEntry: (id: Uuid) => call<void>("delete_entry", { id }),
  revealField: (id: Uuid, field: EntryField) => call<string>("reveal_field", { id, field }),
  copyField: (id: Uuid, field: EntryField) => call<void>("copy_field", { id, field }),
  getEntryHistory: (id: Uuid) => call<RevisionSummary[]>("get_entry_history", { id }),
  restoreRevision: (id: Uuid, index: number) => call<void>("restore_revision", { id, index }),
  copyRevisionPassword: (id: Uuid, index: number) => call<void>("copy_revision_password", { id, index }),
  openEntryUrl: (id: Uuid) => call<void>("open_entry_url", { id }),

  // Folders and icons
  createFolder: (parentId: Uuid | null, name: string, icon: Icon | null) =>
    call<Uuid>("create_folder", { parentId, name, icon }),
  updateFolder: (id: Uuid, name: string, icon: Icon | null) => call<void>("update_folder", { id, name, icon }),
  moveFolder: (id: Uuid, parentId: Uuid | null) => call<void>("move_folder", { id, parentId }),
  deleteFolder: (id: Uuid) => call<void>("delete_folder", { id }),
  addCustomIcon: () => call<Uuid | null>("add_custom_icon"),

  // Generator and clipboard
  generatePassword: (options: GeneratorOptions) => call<string>("generate_password", { options }),
  copySecretText: (text: string) => call<void>("copy_secret_text", { text }),

  // Import, export and merge
  importCsv: (path: string) => call<ImportSummary>("import_csv", { path }),
  importKdbx: (path: string, password: string, keyFile: string | null) =>
    call<ImportSummary>("import_kdbx", { path, password, keyFile }),
  exportCsv: (path: string) => call<void>("export_csv", { path }),
  exportKdbx: (path: string, password: string, keyFile: string | null) =>
    call<void>("export_kdbx", { path, password, keyFile }),
  mergeVaultFile: (path: string, password: string | null, keyFile: string | null) =>
    call<MergeReport>("merge_vault_file", { path, password, keyFile }),
};
