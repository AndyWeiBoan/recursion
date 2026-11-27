// Every icon in the app, from one set (Lucide, ISC licence, lucide.dev) with one stroke, so they
// read as one family. Use them by name; Claude's and Codex's own marks are not icons and stay
// as they are (see KIND in WorkspaceView).
import {
  ArrowDown,
  ArrowLeft,
  ArrowUp,
  Bold,
  ChartColumn,
  Check,
  ChevronDown,
  ChevronRight,
  Code,
  Columns2,
  Copy,
  CornerDownRight,
  CornerUpLeft,
  Ellipsis,
  ExternalLink,
  File,
  FilePen,
  Gauge,
  FileText,
  Folder,
  Globe,
  GripVertical,
  Hand,
  History,
  Image,
  Info,
  Italic,
  KeyRound,
  Keyboard,
  Layers,
  List,
  ListOrdered,
  LoaderCircle,
  LocateFixed,
  MessageSquare,
  MessagesSquare,
  Minus,
  Moon,
  NotebookText,
  PanelLeft,
  Paperclip,
  Plug,
  Plus,
  Reply,
  RotateCcw,
  RotateCw,
  Rows2,
  Settings,
  Square,
  SquareCheck,
  SquareCode,
  SquareTerminal,
  Strikethrough,
  Sun,
  TriangleAlert,
  User,
  Workflow,
  X,
  Zap,
  type LucideIcon,
} from "lucide-react";
import claudePng from "./assets/claude.png";
import openaiPng from "./assets/openai.png";

const ICONS = {
  // the chat box toolbar
  bold: Bold,
  italic: Italic,
  strike: Strikethrough,
  code: Code,
  codeBlock: SquareCode,
  bullets: List,
  numbers: ListOrdered,
  poll: ChartColumn,
  attach: Paperclip,
  // things
  folder: Folder,
  file: File,
  doc: FileText,
  diagram: Workflow,
  page: Globe,
  todo: SquareCheck,
  thread: MessageSquare,
  image: Image,
  note: NotebookText,
  outputs: Layers,
  // panels
  terminal: SquareTerminal,
  chat: MessagesSquare,
  browser: Globe,
  sidebar: PanelLeft,
  splitRight: Columns2,
  splitDown: Rows2,
  // actions
  close: X,
  add: Plus,
  minus: Minus,
  more: Ellipsis,
  grip: GripVertical,
  copy: Copy,
  check: Check,
  reload: RotateCw,
  rewind: RotateCcw,
  reply: Reply,
  bring: CornerUpLeft,
  quote: CornerDownRight,
  result: CornerDownRight,
  open: ExternalLink,
  find: LocateFixed,
  back: ArrowLeft,
  up: ArrowUp,
  down: ArrowDown,
  stop: Square,
  expand: ChevronRight,
  dropdown: ChevronDown,
  recent: History,
  // state
  busy: LoaderCircle,
  key: KeyRound,
  warn: TriangleAlert,
  light: Sun,
  dark: Moon,
  // what an agent may do without asking (PermMode)
  ask: Hand,
  edits: FilePen,
  auto: Zap,
  // settings
  usage: Gauge,
  settings: Settings,
  user: User,
  plug: Plug,
  keyboard: Keyboard,
  info: Info,
} satisfies Record<string, LucideIcon>;

export type IconName = keyof typeof ICONS;

/** `busy` spins (the `.spin` of the old ✳). A size in px, or "1em" to follow the text it sits in. */
export function Icon({ name, size = 16, className }: { name: IconName; size?: number | string; className?: string }) {
  const C = ICONS[name];
  return <C size={size} strokeWidth={2} aria-hidden="true" className={`ic${name === "busy" ? " spin" : ""}${className ? ` ${className}` : ""}`} />;
}

/** The agents' makers' marks as round badges (Claude's, and OpenAI's for Codex), the pictures
 * the human picked: not Lucide, they stand for who the agent is. */
const BRANDS = { claude: claudePng, codex: openaiPng };

export function BrandMark({ kind, size = 16, className }: { kind: keyof typeof BRANDS; size?: number; className?: string }) {
  return <img src={BRANDS[kind]} width={size} height={size} alt="" draggable={false} className={`ic brand${className ? ` ${className}` : ""}`} />;
}
