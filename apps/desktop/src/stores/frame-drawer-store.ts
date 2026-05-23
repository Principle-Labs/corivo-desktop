import { create } from "zustand";

/**
 * Tracks which frame is open in the right-side Frame Drawer (if any).
 * Replaces the `/timeline` master-detail screen — frames are now an
 * inline reference surface that opens from cited context chips in /ask.
 *
 * Usage:
 *   const open = useFrameDrawerStore((s) => s.open);
 *   open(frameId);
 */
type FrameDrawerState = {
  /** `null` ↔ drawer closed. */
  frameId: string | null;

  open: (frameId: string) => void;
  close: () => void;
};

export const useFrameDrawerStore = create<FrameDrawerState>((set) => ({
  frameId: null,
  open: (frameId) => set({ frameId }),
  close: () => set({ frameId: null }),
}));
