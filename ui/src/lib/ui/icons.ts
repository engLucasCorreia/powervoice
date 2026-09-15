/**
 * The icon vocabulary (H-25, A-017: Lucide via `lucide-svelte`). One concept → one icon, app-wide:
 * components ask for a semantic name (`"returnToStart"`), never a Lucide file name, so swapping a
 * glyph is a one-line change here and two features can't pick different icons for the same
 * action. Per-icon imports keep the bundle to the ~60 icons actually used.
 */
import Activity from "@lucide/svelte/icons/activity";
import AudioLines from "@lucide/svelte/icons/audio-lines";
import AudioWaveform from "@lucide/svelte/icons/audio-waveform";
import Ban from "@lucide/svelte/icons/ban";
import ChartSpline from "@lucide/svelte/icons/chart-spline";
import Check from "@lucide/svelte/icons/check";
import ChevronDown from "@lucide/svelte/icons/chevron-down";
import ChevronLeft from "@lucide/svelte/icons/chevron-left";
import ChevronRight from "@lucide/svelte/icons/chevron-right";
import ChevronsLeft from "@lucide/svelte/icons/chevrons-left";
import ChevronsRight from "@lucide/svelte/icons/chevrons-right";
import ChevronUp from "@lucide/svelte/icons/chevron-up";
import Circle from "@lucide/svelte/icons/circle";
import CircleQuestionMark from "@lucide/svelte/icons/circle-question-mark";
import CircleAlert from "@lucide/svelte/icons/circle-alert";
import CircleCheck from "@lucide/svelte/icons/circle-check";
import ClipboardPaste from "@lucide/svelte/icons/clipboard-paste";
import Copy from "@lucide/svelte/icons/copy";
import Ellipsis from "@lucide/svelte/icons/ellipsis";
import EllipsisVertical from "@lucide/svelte/icons/ellipsis-vertical";
import Eye from "@lucide/svelte/icons/eye";
import EyeOff from "@lucide/svelte/icons/eye-off";
import Flag from "@lucide/svelte/icons/flag";
import FolderOpen from "@lucide/svelte/icons/folder-open";
import FolderPlus from "@lucide/svelte/icons/folder-plus";
import FolderSearch from "@lucide/svelte/icons/folder-search";
import Gauge from "@lucide/svelte/icons/gauge";
import GripVertical from "@lucide/svelte/icons/grip-vertical";
import HardDrive from "@lucide/svelte/icons/hard-drive";
import Headphones from "@lucide/svelte/icons/headphones";
import Info from "@lucide/svelte/icons/info";
import Keyboard from "@lucide/svelte/icons/keyboard";
import Layers from "@lucide/svelte/icons/layers";
import LoaderCircle from "@lucide/svelte/icons/loader-circle";
import Lock from "@lucide/svelte/icons/lock";
import Mic from "@lucide/svelte/icons/mic";
import MicOff from "@lucide/svelte/icons/mic-off";
import Minus from "@lucide/svelte/icons/minus";
import PackagePlus from "@lucide/svelte/icons/package-plus";
import PanelBottom from "@lucide/svelte/icons/panel-bottom";
import PanelLeft from "@lucide/svelte/icons/panel-left";
import PanelRight from "@lucide/svelte/icons/panel-right";
import Pause from "@lucide/svelte/icons/pause";
import Play from "@lucide/svelte/icons/play";
import Plug from "@lucide/svelte/icons/plug";
import Plus from "@lucide/svelte/icons/plus";
import Power from "@lucide/svelte/icons/power";
import Redo2 from "@lucide/svelte/icons/redo-2";
import RefreshCw from "@lucide/svelte/icons/refresh-cw";
import Repeat from "@lucide/svelte/icons/repeat";
import RotateCcw from "@lucide/svelte/icons/rotate-ccw";
import Save from "@lucide/svelte/icons/save";
import Scissors from "@lucide/svelte/icons/scissors";
import Search from "@lucide/svelte/icons/search";
import Settings2 from "@lucide/svelte/icons/settings-2";
import SkipBack from "@lucide/svelte/icons/skip-back";
import SkipForward from "@lucide/svelte/icons/skip-forward";
import SlidersHorizontal from "@lucide/svelte/icons/sliders-horizontal";
import Split from "@lucide/svelte/icons/split";
import Square from "@lucide/svelte/icons/square";
import Timer from "@lucide/svelte/icons/timer";
import Trash2 from "@lucide/svelte/icons/trash-2";
import TriangleAlert from "@lucide/svelte/icons/triangle-alert";
import Undo2 from "@lucide/svelte/icons/undo-2";
import Volume2 from "@lucide/svelte/icons/volume-2";
import WandSparkles from "@lucide/svelte/icons/wand-sparkles";
import X from "@lucide/svelte/icons/x";
import ZoomIn from "@lucide/svelte/icons/zoom-in";
import ZoomOut from "@lucide/svelte/icons/zoom-out";

export type LucideIcon = typeof Play;

export const ICONS = {
  // Transport
  play: Play,
  pause: Pause,
  stop: Square,
  record: Circle,
  returnToStart: SkipBack,
  playFromStart: RotateCcw,
  goToEnd: SkipForward,
  loop: Repeat,
  punch: Split,
  // Recording / devices
  input: Mic,
  inputOff: MicOff,
  monitor: Headphones,
  output: Volume2,
  disk: HardDrive,
  timer: Timer,
  // Views
  waveform: AudioWaveform,
  spectral: AudioLines,
  analyzer: ChartSpline,
  loudness: Gauge,
  meters: Activity,
  zoomIn: ZoomIn,
  zoomOut: ZoomOut,
  panelLeft: PanelLeft,
  panelRight: PanelRight,
  panelBottom: PanelBottom,
  // Editing
  marker: Flag,
  cut: Scissors,
  copy: Copy,
  paste: ClipboardPaste,
  undo: Undo2,
  redo: Redo2,
  add: Plus,
  remove: Minus,
  delete: Trash2,
  // Rack
  rack: Layers,
  effects: SlidersHorizontal,
  bypass: Power,
  cleanup: WandSparkles,
  drag: GripVertical,
  // Plugins (T-809)
  plugin: Plug,
  install: PackagePlus,
  blocked: Ban,
  refresh: RefreshCw,
  folderAdd: FolderPlus,
  reveal: FolderSearch,
  // Files / app
  open: FolderOpen,
  save: Save,
  settings: Settings2,
  keyboard: Keyboard,
  search: Search,
  lock: Lock,
  visible: Eye,
  hidden: EyeOff,
  // Disclosure / navigation
  more: Ellipsis,
  moreVertical: EllipsisVertical,
  chevronDown: ChevronDown,
  chevronUp: ChevronUp,
  chevronLeft: ChevronLeft,
  chevronRight: ChevronRight,
  collapseLeft: ChevronsLeft,
  collapseRight: ChevronsRight,
  close: X,
  check: Check,
  // Status
  info: Info,
  help: CircleQuestionMark,
  success: CircleCheck,
  warning: TriangleAlert,
  error: CircleAlert,
  loading: LoaderCircle,
} satisfies Record<string, LucideIcon>;

export type IconName = keyof typeof ICONS;

export const ICON_NAMES = Object.keys(ICONS) as IconName[];

/** Icon sizes in px, matching `--pv-icon-sm/md/lg` (14/16/20). */
export const ICON_SIZE_PX = { sm: 14, md: 16, lg: 20 } as const;
export type IconSize = keyof typeof ICON_SIZE_PX;
