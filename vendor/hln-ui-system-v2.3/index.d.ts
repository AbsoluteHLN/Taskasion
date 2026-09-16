// ProjectHLN UI System v2.3 - TypeScript Declarations
// Industrial-Tactical-Vector curated subset of v2.

export type HlnV23Theme =
  | "arknights"
  | "endfield"
  | "blacksteel"
  | "abyss-aegir"
  | "babel"
  | "monster-siren"

export type HlnV23Font =
  | "display"
  | "technical"
  | "grotesque"
  | "cyber"
  | "berlin"
  | "condensed"

export type HlnV23MotionPreset = "panel" | "item" | "control"
export type HlnV23MotionState = "enter" | "exit"
export type HlnV23MotionVariant =
  | "cyber-scan"
  | "wipe-reveal"
  | "radar-sweep"
  | "clip-scan"
  | "tactical-lock"
  | "data-stream"
  | "grid-march"
  | "scanline-decode"
  | "corner-deploy"
  | "radar-deploy"
  | "pulse-chain"
  | "signal-sweep"

export type HlnV23BgPresetId =
  | "tactical-grid"
  | "holo-scan"
  | "circuit-trace"
  | "prts-sonar"
  | "hazard-hatch"
  | "hex-field"
  | "radar-cross"
  | "data-rain"
  | "blueprint"
  | "data-lattice"
  | "blueprint-schematic"
  | "quiet"

export interface HlnV23ThemeMetadata {
  id: HlnV23Theme
  label: string
  swatch: string
  accent: string
  colorScheme: "dark" | "light"
}

export interface HlnV23BgPreset {
  id: HlnV23BgPresetId
  label: string
  note: string
}

export declare const HLN_V23_THEMES: readonly HlnV23Theme[]
export declare const HLN_V23_THEME_REGISTRY: readonly HlnV23ThemeMetadata[]
export declare const HLN_V23_BG_PRESETS: readonly HlnV23BgPreset[]
export declare const HLN_V23_FONTS: readonly HlnV23Font[]

export declare function isHlnV23Theme(theme: string): theme is HlnV23Theme
export declare function applyHlnV23Theme(theme: HlnV23Theme, target?: HTMLElement | null): boolean
export declare function currentHlnV23Theme(target?: HTMLElement | null): HlnV23Theme

export declare function isHlnV23Font(font: string): font is HlnV23Font
export declare function applyHlnV23Font(font: HlnV23Font, target?: HTMLElement | null): boolean
export declare function currentHlnV23Font(target?: HTMLElement | null): HlnV23Font

export declare function playHlnV23Motion(
  scope: HTMLElement | null,
  preset?: HlnV23MotionPreset,
  state?: HlnV23MotionState,
  variant?: HlnV23MotionVariant
): void
