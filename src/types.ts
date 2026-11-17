// Shapes exchanged with the Rust core. They mirror `views.rs`, `models.rs` and `settings.rs`.

export type Uuid = string;

/** Core session: prefer UI wording “No vault” / “Locked” / “Unlocked” (not “Closed”). */
export type SessionState = "closed" | "locked" | "unlocked";
export type Theme = "system" | "light" | "dark";
export type LockReason = "user" | "inactivity" | "tray";
export type FileKind = "vault" | "keyFile" | "csv" | "kdbx" | "image" | "recovery";

/** Which browsers are installed on this machine (drives the extension install flow). */
export interface BrowserPresence {
  firefox: boolean;
  chrome: boolean;
  edge: boolean;
}

export type ErrorKind =
  | "invalidCredentials"
  | "sessionExpired"
  | "notAVault"
  | "unsupportedVersion"
  | "corrupted"
  | "locked"
  | "noVaultSelected"
  | "notFound"
  | "invalid"
  | "import"
  | "export"
  | "io"
  | "serialization"
  | "crypto"
  | "system"
  | "unknown";

export interface GeneratorOptions {
  length: number;
  lowercase: boolean;
  uppercase: boolean;
  digits: boolean;
  symbols: boolean;
  excludeAmbiguous: boolean;
  spaces: boolean;
  excludeCharacters: string;
  beginWithLetter: boolean;
  noRepeating: boolean;
  requireEachType: boolean;
  minLowercase: number;
  minUppercase: number;
  minDigits: number;
  minSymbols: number;
}

export interface Preferences {
  theme: Theme;
  autoLockMinutes: number;
  clipboardClearSeconds: number;
  closeToTray: boolean;
  /** Folder new vaults are created in and downloads land in; null = the default. */
  vaultsFolder: string | null;
  generator: GeneratorOptions;
}

export interface RecentVault {
  path: string;
  fileName: string;
  keyFile: string | null;
  exists: boolean;
  /** This vault is set up for cloud sync. */
  synced: boolean;
  /** Last successful sync, in Unix milliseconds. */
  lastSyncedAt: number | null;
}

export interface AppStatus {
  state: SessionState;
  vaultPath: string | null;
  vaultFileName: string | null;
  keyFile: string | null;
  preferences: Preferences;
  recentVaults: RecentVault[];
  appVersion: string;
  /** Whether the open vault can also be opened with a recovery key. */
  hasRecoveryKey: boolean;
  /** Cloud sync state. */
  sync: SyncView;
}

/** Cloud sync state: whether the user is signed in and whether the open vault syncs. */
export interface SyncView {
  loggedIn: boolean;
  email: string | null;
  serverUrl: string | null;
  thisVaultSynced: boolean;
  /** Last successful sync of the open vault, in Unix milliseconds. */
  lastSyncedAt: number | null;
  /** When the account master password last changed, in Unix milliseconds. */
  passwordChangedAt: number | null;
  /** Entitlement: "comp" | "trialing" | "active" | "past_due" | "canceled" | "expired". */
  plan: string | null;
  /** End of the current paid or trial period, in Unix milliseconds. */
  planValidUntil: number | null;
  /** Cloud vaults known to this device, grouped by the vault's own identity. */
  cloud: CloudVaultView[];
  /** Vaults on this device that are not synced. */
  local: LocalVaultView[];
}

/** A vault that has a cloud copy, with the local copies on this device. */
export interface CloudVaultView {
  name: string;
  /** The vault's own id (immutable): the true identity of the vault. */
  localVaultId: string;
  serverVaultId: string;
  lastSyncedAt: number | null;
  /** Local file paths of this vault on this device. */
  copies: string[];
  /** At least one local copy exists on disk. */
  present: boolean;
}

/** A vault on this device that is not synced. */
export interface LocalVaultView {
  path: string;
  fileName: string;
  keyFile: string | null;
}

/** A cloud vault as listed by the server (metadata only). */
export interface CloudVault {
  id: string;
  name: string;
  /** The encrypted label; absent on vaults uploaded by older versions. */
  nameCipher: string | null;
  vaultUuid: string | null;
  version: number;
  updatedAt: number;
}

export type Icon = { kind: "builtin"; value: string } | { kind: "custom"; value: Uuid };

export interface FolderView {
  id: Uuid;
  parentId: Uuid | null;
  name: string;
  icon: Icon | null;
}

export interface EntrySummary {
  id: Uuid;
  folderId: Uuid | null;
  title: string;
  username: string;
  url: string | null;
  icon: Icon | null;
  /** `YYYY-MM-DD` */
  expirationDate: string | null;
  modifiedAt: string;
  hasPassword: boolean;
}

export type UrlMatchKind = "exactHost" | "parentHost";

/** Autofill candidate for a page URL (no secrets). */
export interface UrlMatchHit {
  entry: EntrySummary;
  matchKind: UrlMatchKind;
}

export interface CustomIconView {
  id: Uuid;
  mime: string;
  /** Base64 image bytes. */
  data: string;
}

export interface VaultView {
  name: string;
  path: string;
  fileName: string;
  folders: FolderView[];
  entries: EntrySummary[];
  customIcons: CustomIconView[];
}

export interface CustomFieldView {
  name: string;
  /** `null` for protected fields until the user reveals them. */
  value: string | null;
  protected: boolean;
}

export interface EntryDetails {
  id: Uuid;
  folderId: Uuid | null;
  folderPath: string[];
  title: string;
  username: string;
  hasPassword: boolean;
  url: string | null;
  notes: string | null;
  expirationDate: string | null;
  icon: Icon | null;
  customFields: CustomFieldView[];
  createdAt: string;
  modifiedAt: string;
  historyCount: number;
}

export interface CustomField {
  name: string;
  value: string;
  protected: boolean;
}

/** The editable content of an entry, secrets included. Only the entry editor holds one. */
export interface EntryContent {
  title: string;
  username: string;
  password: string;
  url: string | null;
  notes: string | null;
  expirationDate: string | null;
  icon: Icon | null;
  customFields: CustomField[];
}

export interface RevisionSummary {
  index: number;
  modifiedAt: string;
  title: string;
  username: string;
  url: string | null;
  passwordDiffers: boolean;
}

export interface ImportSummary {
  folderName: string;
  entriesImported: number;
  foldersCreated: number;
  duplicatesSkipped: number;
  rowsSkipped: number;
  attachmentsSkipped: number;
}

export interface MergeReport {
  entriesAdded: number;
  entriesUpdated: number;
  entriesDeleted: number;
  conflictsResolved: number;
  duplicatesSkipped: number;
  foldersAdded: number;
  foldersUpdated: number;
  foldersDeleted: number;
}

export type EntryField =
  | { kind: "username" }
  | { kind: "password" }
  | { kind: "url" }
  | { kind: "notes" }
  | { kind: "custom"; index: number };
