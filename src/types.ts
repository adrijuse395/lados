// Shapes exchanged with the Rust core. They mirror `views.rs`, `models.rs` and `settings.rs`.

export type Uuid = string;

export type SessionState = "closed" | "locked" | "unlocked";
export type Theme = "system" | "light" | "dark";
export type LockReason = "user" | "inactivity" | "tray";
export type FileKind = "vault" | "keyFile" | "csv" | "kdbx" | "image";

export type ErrorKind =
  | "invalidCredentials"
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
  generator: GeneratorOptions;
}

export interface RecentVault {
  path: string;
  fileName: string;
  keyFile: string | null;
  exists: boolean;
}

export interface AppStatus {
  state: SessionState;
  vaultPath: string | null;
  vaultFileName: string | null;
  keyFile: string | null;
  preferences: Preferences;
  recentVaults: RecentVault[];
  appVersion: string;
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
