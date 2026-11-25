// The IPC contract module: the ONLY place that imports the generated
// bindings (D-003). Everything else in the frontend reaches Tauri through
// `src/ipc` (this re-export), which is what lets a future ESLint pin forbid
// `../bindings` imports outside `src/ipc/**` without touching callers.
//
// The generated `commands` object is re-exported as `realCommands` for the
// backend switch in `src/ipc/index.ts`; the derived `IpcCommands` type is
// the facade's contract — a mock backend is a drop-in replacement because
// it is annotated with the very same type (every command, exact signature).

import { commands } from '../bindings'

/** The generated Tauri command surface, as the frontend must see it. */
export type IpcCommands = typeof commands

/** A command's wire name in camelCase, e.g. `projectsList`. */
export type CommandName = keyof IpcCommands

/**
 * The real backend: the tauri-specta generated `commands`. Calls leave the
 * webview through `invoke` (inside `src/bindings.ts`, the only permitted
 * spot — no component calls `invoke()` directly).
 */
export const realCommands: IpcCommands = commands

// Wire types the mock layer produces. Only the `*_Serialize` halves are
// re-exported: those are the shapes the Rust side actually puts on the wire
// (the generated function return types), so the mock data matches them.
// The event payload types follow the same rule since the first generated
// event landed (T-079/D-112): the Rust side emits the `*_Serialize` half,
// so that is the shape the mock bus broadcasts.
export type {
  ActivityEntry_Serialize as ActivityEntryWire,
  ActivityListQuery_Serialize as ActivityListQueryWire,
  ActivityPage_Serialize as ActivityPageWire,
  AddedProject_Serialize as AddedProjectWire,
  AssignmentRecord as AssignmentRecordWire,
  AssignmentTargetRecord as AssignmentTargetWire,
  BackupCopyRecord_Serialize as BackupCopyRecordWire,
  BulkAssignmentResult as BulkAssignmentResultWire,
  AppError_Serialize as AppErrorWire,
  AppInfo,
  DbBackupInfo,
  ErrorCode,
  ExclusionEntry,
  ExistingProject,
  HeadingRecord as HeadingRecordWire,
  HeadingUpdate,
  InstructionCreated,
  LibraryHeadingEntry,
  LibraryInstructionEntry,
  LibraryTreeItem,
  LibraryGetResult_Serialize as LibraryGetResultWire,
  LibraryTreeItem_Serialize as LibraryTreeItemWire,
  LibraryVariantEntry,
  MarkdownPreview,
  MoveTarget,
  ProjectAddResult_Serialize as ProjectAddResultWire,
  ProjectAvailabilityEvent_Serialize as ProjectAvailabilityEventWire,
  ProjectColumnEntry_Serialize as ProjectColumnEntryWire,
  ProjectLabelEntry_Serialize as ProjectLabelEntry,
  RevisionRecord_Serialize as RevisionRecordWire,
  RevisionSummaryRecord_Serialize as RevisionSummaryRecordWire,
  ScanClassWire,
  ScanCommitEntry_Serialize as ScanCommitEntryWire,
  ScanCommitResult_Serialize as ScanCommitResultWire,
  ScanProgressEvent as ScanProgressEventWire,
  ScanRunOutcome_Serialize as ScanRunOutcomeWire,
  ScanSkipReason,
  ScanSkip_Serialize as ScanSkipWire,
  ScannedRepository,
  SettingChange,
  SettingItem,
  VariantRecord_Serialize as VariantRecordWire,
  VariantSource,
} from '../bindings'
